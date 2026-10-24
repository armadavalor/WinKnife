//! `#[tauri::command]` entry points.
//!
//! Deliberately thin: no filesystem, no registry, no SQL here — commands only
//! translate between IPC and `core`/`modules` and map errors to [`ApiError`]
//! (CONVENTIONS.md). The one thing they do own is the *composition*: which
//! quarantine, which brain, which recycle-bin wrapper a run uses. That is the
//! seam that keeps the real Win32 calls out of every test.
//!
//! # Managed state
//!
//! Every `State<'_, T>` below names the type `lib.rs` passes to `app.manage`,
//! **exactly**. Tauri resolves managed state by `TypeId`, so `Arc<Database>` and
//! `Database` are two unrelated keys: registering one and asking for the other
//! compiles clean and then fails on every single invocation with
//! `state not managed for field ...`. That is why the database arrives here as
//! `Arc<Database>` — the `Arc` is what gets registered. `tests/command_wiring.rs` invokes every registered
//! command against a real app and fails if these ever drift apart again.
//!
//! # Threads
//!
//! A plain `#[tauri::command]` runs on the window's own thread. Anything that
//! waits on the database lock, the disk or another process is
//! `#[tauri::command(async)]` instead: while a batch or a scan holds the lock,
//! a click that needed it used to wait there too and the window went
//! "Not Responding".

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use tauri::{AppHandle, Runtime, State};
use uuid::Uuid;

use crate::core::brain::Brain;
use crate::core::busy::Operation;
use crate::core::db::Database;
use crate::core::error::{ApiError, CoreError};
use crate::core::events::{
    self, ExecProgress, ProgressThrottle, QuarantineChanged, ScanDone, ScanProgress,
};
use crate::core::executor::{self, ExecEnvironment};
use crate::core::history;
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
use crate::modules::duplicates::{self, folders, DuplicateProgress, DuplicateScan};
use crate::modules::empty_folders;
use crate::modules::hosts::{HostsBackup, HostsDocument, HostsPreview, HostsService, SaveOutcome};
use crate::modules::keep_awake::{KeepAwake, KeepAwakeStatus};
use crate::modules::legacy_panels::{LegacyPanelsService, PanelInfo};
use crate::modules::restore_point::{CreateOutcome, RestorePointService, RestorePointStatus};
use crate::modules::shutdown_timer::{ScheduleMode, ShutdownTimer, TimerStatus};
use crate::modules::{context_menu, startup, treemap, tweaks};

/// Run a sync command body, turning a panic into an `internal` [`ApiError`].
///
/// Without this a panic in a command never answers the WebView: the page that
/// awaited the call spins forever (issue #40). Every command body runs inside
/// this or [`guarded_async`]; `tests/panic_and_busy.rs` checks that.
pub fn guarded<T>(
    command: &'static str,
    body: impl FnOnce() -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .unwrap_or_else(|payload| Err(panic_error(command, payload.as_ref())))
}

/// The async twin of [`guarded`]: each poll of `body` runs under
/// `catch_unwind`, so a panic anywhere in the future answers with an error
/// instead of killing the task that owns the IPC response.
pub async fn guarded_async<T, F>(command: &'static str, body: F) -> Result<T, ApiError>
where
    F: std::future::Future<Output = Result<T, ApiError>>,
{
    let mut body = std::pin::pin!(body);
    std::future::poll_fn(|cx| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body.as_mut().poll(cx)))
            .unwrap_or_else(|payload| {
                std::task::Poll::Ready(Err(panic_error(command, payload.as_ref())))
            })
    })
    .await
}

/// The error a caught panic becomes. The panic text goes to the log and to
/// `detail`; the message stays readable.
fn panic_error(command: &'static str, payload: &(dyn std::any::Any + Send)) -> ApiError {
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-text panic payload".to_owned());
    tracing::error!(command, panic = %text, "command panicked");
    ApiError::new(
        "internal",
        format!(
            "WinKnife hit an internal error while running `{command}`. \
             The operation stopped; see the log for details."
        ),
    )
    .with_detail(text)
}

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
    /// Autostart entries, for `Action::DisableStartupEntry`.
    pub startup: Arc<dyn executor::StartupControl>,
    /// The same service, for the enumeration and the re-enable path.
    pub startup_service: Arc<startup::StartupService>,
    /// Per-user settings, for `Action::ApplyTweak` and its undo (K-018).
    pub tweaks: Arc<tweaks::TweakService>,
    /// Per-user context-menu entries, disabled through the same undo table.
    pub context_menu: Arc<context_menu::ContextMenuService>,
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
    guarded("list_modules", || Ok(registry.manifests()))
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
    db: State<'_, Arc<Database>>,
    cancels: State<'_, CancelRegistry>,
) -> Result<ScanResult, ApiError> {
    guarded_async("start_scan", async {
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
        let started_at = Utc::now();
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
                    items_total: None,
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

        tracing::info!(
            scan = %scan_id,
            findings = output.findings.len(),
            cancelled = output.cancelled,
            "scan walk finished, analysing"
        );

        // Judging every finding is the long half on a full `%TEMP%` (a path
        // check and a lock probe each), so it reports progress against a known
        // total and stays cancellable: the token is only released after it.
        let analysed = {
            let mut throttle = ProgressThrottle::new();
            let mut emit = |done: u64, total: u64, path: &Path| {
                // The final tick always goes out; the rest are throttled.
                if done != total && !throttle.should_emit() {
                    return;
                }
                let payload = ScanProgress {
                    scan_id,
                    module: module_id.clone(),
                    items_scanned: done,
                    current_path: Some(path.display().to_string()),
                    pct: percent(done, total),
                    items_total: Some(total),
                };
                if let Err(err) = events::emit_scan_progress(&app, &payload) {
                    tracing::debug!(error = %err, "scan progress event was not delivered");
                }
            };
            cleaner::analyze_with(
                &output.findings,
                &services.brain,
                Utc::now(),
                &cancel,
                Some(&mut emit),
            )
        };

        cancels.finish(scan_id);

        // On a stop, only the judged prefix is kept: a finding without its
        // recommendation must never reach the review screen.
        let mut findings = output.findings;
        findings.truncate(analysed.analyzed);
        let result = ScanResult {
            scan_id,
            module: module_id.clone(),
            findings,
            recommendations: analysed.recommendations,
            skipped_rules: output.skipped_rules,
            cancelled: output.cancelled || analysed.cancelled,
        };

        tracing::info!(
            scan = %scan_id,
            findings = result.findings.len(),
            unreadable = output.unreadable.len(),
            cancelled = result.cancelled,
            "scan finished"
        );

        remember(
            &db,
            scan_id,
            "cleaner",
            started_at,
            result.findings.len() as u64,
            result.safe_bytes() + result.review_bytes(),
            result.cancelled,
        );

        let done = ScanDone {
            scan_id,
            module: module_id,
            finding_count: result.findings.len() as u64,
        };
        let _ = events::emit_scan_done(&app, &done);

        store.insert(result.clone());
        Ok(result)
    })
    .await
}

/// Fetch a scan the UI already ran.
///
/// # Errors
/// Returns [`CoreError::UnknownScan`] when the scan is no longer retained.
#[tauri::command(async)]
pub fn get_recommendations(
    scan_id: Uuid,
    store: State<'_, ScanStore>,
) -> Result<ScanResult, ApiError> {
    guarded("get_recommendations", || Ok(store.get(scan_id)?))
}

/// Green/yellow/red totals for the header line after a scan.
///
/// # Errors
/// See [`get_recommendations`].
#[tauri::command(async)]
pub fn scan_summary(
    scan_id: Uuid,
    store: State<'_, ScanStore>,
) -> Result<CleanerSummary, ApiError> {
    guarded("scan_summary", || {
        let scan = store.get(scan_id)?;
        Ok(cleaner::summarise(&scan.recommendations))
    })
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
// Tauri injects each piece of managed state as its own argument.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn execute<R: Runtime>(
    app: AppHandle<R>,
    scan_id: Uuid,
    approved: Vec<Uuid>,
    confirm_empty_recycle_bin: bool,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
    db: State<'_, Arc<Database>>,
    cancels: State<'_, CancelRegistry>,
) -> Result<ExecReport, ApiError> {
    guarded_async("execute", async {
        // Before the database lock: a second click must be refused, not queued.
        let _busy = services.quarantine.begin(Operation::Clean)?;
        // Stop goes through `cancel_scan` with the same id (issue #39).
        let ticket = cancels.track(scan_id);
        let scan = store.get(scan_id)?;
        let units = executor::plan(&approved, &scan.recommendations, &scan.findings);

        let mut throttle = ProgressThrottle::new();
        let module = scan.module.clone();
        let tweak_router = context_menu::TweakRouter {
            tweaks: services.tweaks.as_ref(),
            context_menu: services.context_menu.as_ref(),
        };
        let env = ExecEnvironment {
            quarantine: &services.quarantine,
            db: &**db,
            brain: &services.brain,
            recycle_bin: services.recycle_bin.as_ref(),
            startup: services.startup.as_ref(),
            tweaks: &tweak_router,
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
        let report = executor::execute(
            &units,
            &env,
            &executor::ExecOptions {
                confirm_empty_recycle_bin,
                cancel: Some(ticket.token()),
            },
            Some(&mut emit),
        )?;

        notify_quarantine_changed(&app, &services, &db, report.batch_id);
        Ok(report)
    })
    .await
}

/// Ask a running scan to stop (ARCHITECTURE section 3).
///
/// Returns `false` when the scan already finished, which is not an error — the
/// user pressed stop as the last file went through.
///
/// # Errors
/// Infallible today; keeps the shared `Result` shape.
#[tauri::command(async)]
pub fn cancel_scan(scan_id: Uuid, cancels: State<'_, CancelRegistry>) -> Result<bool, ApiError> {
    guarded("cancel_scan", || {
        let stopped = cancels.cancel(scan_id);
        tracing::info!(scan = %scan_id, stopped, "cancel requested");
        Ok(stopped)
    })
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
    /// Groups of identical folders, keeper first in each.
    #[serde(default)]
    pub folder_groups: Vec<folders::FolderGroup>,
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
    db: State<'_, Arc<Database>>,
    cancels: State<'_, CancelRegistry>,
) -> Result<DuplicateScanResult, ApiError> {
    guarded_async("start_duplicate_scan", async {
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
        let started_at = Utc::now();
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
                items_total: None,
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
        // A Stop accepted after the pipeline's last check was still promised
        // to the UI (`cancel_scan` said true), so the run is reported stopped.
        let stopped_late = cancels.finish(scan_id);
        let scan = if stopped_late && !scan.cancelled {
            scan.stopped()
        } else {
            scan
        };

        let ignored = db.with(folders::ignored_groups)?;
        let folder_scan = if scan.cancelled {
            folders::FolderScan::default()
        } else {
            folders::scan_folders(
                &duplicates::DuplicateContext {
                    roots: &roots,
                    protected: services.brain.protected_paths(),
                    settings: &stored,
                    cancel: &cancel,
                },
                &ignored,
                Some(&emit),
            )
        };
        let (folder_groups, folder_recs) =
            folders::analyze(&folder_scan.findings, &services.brain, &HashMap::new());

        let recommendations = duplicates::analyze(&scan, &services.brain);
        tracing::info!(
            folder_groups = folder_groups.len(),
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
            findings: [scan.findings.clone(), folder_scan.findings].concat(),
            recommendations: [recommendations.clone(), folder_recs.clone()].concat(),
            skipped_rules: scan.skipped_roots.clone(),
            cancelled: scan.cancelled || folder_scan.cancelled,
        });

        remember(
            &db,
            scan_id,
            "duplicates",
            started_at,
            scan.sets.len() as u64,
            scan.wasted_bytes(),
            scan.cancelled,
        );

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
            recommendations: [recommendations, folder_recs].concat(),
            folder_groups,
        })
    })
    .await
}

/// A folder group re-analysed under a new keep rule.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderGroupUpdate {
    /// The group, keeper first.
    pub group: folders::FolderGroup,
    /// Ids of the recommendations this group had before; they are gone now.
    pub replaced: Vec<Uuid>,
    /// The group's recommendations under the new rule.
    pub recommendations: Vec<crate::core::types::Recommendation>,
}

/// Change which copy a folder group keeps. Re-analyses only that group.
///
/// # Errors
/// Returns an error when the scan is no longer retained or the group is not in it.
#[tauri::command]
pub fn set_folder_keep_rule(
    scan_id: Uuid,
    group_id: String,
    rule: folders::KeepRule,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
) -> Result<FolderGroupUpdate, ApiError> {
    guarded("set_folder_keep_rule", || {
        let mut result = store.get(scan_id)?;
        let in_group = |finding: &crate::core::types::Finding| {
            finding.meta.get("folderGroupId").and_then(|id| id.as_str()) == Some(&group_id)
        };
        let group_findings: Vec<_> = result
            .findings
            .iter()
            .filter(|f| in_group(f))
            .cloned()
            .collect();
        let finding_ids: HashSet<Uuid> = group_findings.iter().map(|f| f.id).collect();

        let rules = HashMap::from([(group_id.clone(), rule)]);
        let (groups, recommendations) = folders::analyze(&group_findings, &services.brain, &rules);
        let group = groups.into_iter().next().ok_or_else(|| {
            CoreError::UnknownScan(format!("{scan_id} has no folder group {group_id}"))
        })?;

        let (replaced, kept): (Vec<_>, Vec<_>) = result
            .recommendations
            .into_iter()
            .partition(|rec| rec.finding_ids.iter().any(|id| finding_ids.contains(id)));
        result.recommendations = [kept, recommendations.clone()].concat();
        store.replace(result)?;

        Ok(FolderGroupUpdate {
            group,
            replaced: replaced.into_iter().map(|rec| rec.id).collect(),
            recommendations,
        })
    })
}

/// Leave a folder group out of every later scan.
///
/// # Errors
/// Returns an error if the setting could not be written.
#[tauri::command]
pub fn ignore_folder_group(id: String, db: State<'_, Arc<Database>>) -> Result<(), ApiError> {
    guarded("ignore_folder_group", || {
        Ok(db.with(|conn| folders::ignore_group(conn, &id))?)
    })
}

/// Bring every ignored folder group back.
///
/// # Errors
/// Returns an error if the setting could not be cleared.
#[tauri::command]
pub fn clear_ignored_folder_groups(db: State<'_, Arc<Database>>) -> Result<(), ApiError> {
    guarded("clear_ignored_folder_groups", || {
        Ok(db.with(folders::clear_ignored)?)
    })
}

/// What an empty-folder scan hands the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmptyFolderScanResult {
    /// Id for `execute` and `cancel_scan`.
    pub scan_id: Uuid,
    /// The folder that was scanned.
    pub root: PathBuf,
    /// The topmost empty folders.
    pub findings: Vec<crate::core::types::Finding>,
    /// One explained recommendation per finding.
    pub recommendations: Vec<crate::core::types::Recommendation>,
    /// Folders that could not be read and were therefore treated as not empty.
    pub unreadable: u64,
    /// Protected trees that were deliberately not entered.
    pub skipped_protected: u64,
    /// Folders looked at.
    pub folders_visited: u64,
    /// Whether the scan was cancelled part-way.
    pub cancelled: bool,
}

/// Scan `root` (default: the user profile) for folders with no files in them.
///
/// Blocking work; read-only.
///
/// # Errors
/// Returns an error when the root is not a folder, is a protected location, or
/// no default root can be determined.
#[tauri::command]
pub async fn start_empty_folder_scan<R: Runtime>(
    app: AppHandle<R>,
    root: Option<PathBuf>,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
    db: State<'_, Arc<Database>>,
    cancels: State<'_, CancelRegistry>,
) -> Result<EmptyFolderScanResult, ApiError> {
    guarded_async("start_empty_folder_scan", async {
        use crate::core::platform::EnvResolver;

        let invalid = |message: String| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                message,
            ))
        };
        let root = root
            .or_else(|| {
                crate::core::platform::SystemEnv
                    .var("USERPROFILE")
                    .map(PathBuf::from)
            })
            .ok_or_else(|| {
                invalid("no folder was chosen and no user profile was found".to_owned())
            })?;
        if !root.is_dir() {
            return Err(invalid(format!("`{}` is not a folder", root.display())).into());
        }
        if let Some(refusal) = services.brain.path_refusal(&root) {
            return Err(invalid(refusal.reason).into());
        }

        let scan_id = Uuid::new_v4();
        let started_at = Utc::now();
        let cancel = cancels.start(scan_id);
        let mut throttle = ProgressThrottle::new();
        let mut emit = |visited: u64, current: &Path| {
            if !throttle.should_emit() {
                return;
            }
            let payload = ScanProgress {
                scan_id,
                module: empty_folders::MODULE_ID.to_owned(),
                items_scanned: visited,
                current_path: Some(current.display().to_string()),
                pct: None,
                items_total: None,
            };
            if let Err(err) = events::emit_scan_progress(&app, &payload) {
                tracing::debug!(error = %err, "empty-folder progress event was not delivered");
            }
        };

        // Name the scan to the UI before the walk starts, so a Stop pressed
        // right away has an id to cancel instead of waiting for the first folder.
        let initial = ScanProgress {
            scan_id,
            module: empty_folders::MODULE_ID.to_owned(),
            items_scanned: 0,
            current_path: Some(root.display().to_string()),
            pct: None,
            items_total: None,
        };
        if let Err(err) = events::emit_scan_progress(&app, &initial) {
            tracing::debug!(error = %err, "initial empty-folder progress event was not delivered");
        }

        let output = empty_folders::scan(
            &empty_folders::ScanContext {
                root: &root,
                protected: services.brain.protected_paths(),
                cancel: &cancel,
            },
            Some(&mut emit),
        );
        cancels.finish(scan_id);

        let recommendations = empty_folders::analyze(&output.findings, &services.brain);
        store.insert(ScanResult {
            scan_id,
            module: empty_folders::MODULE_ID.to_owned(),
            findings: output.findings.clone(),
            recommendations: recommendations.clone(),
            skipped_rules: Vec::new(),
            cancelled: false,
        });
        remember(
            &db,
            scan_id,
            empty_folders::MODULE_ID,
            started_at,
            output.findings.len() as u64,
            0,
            output.cancelled,
        );
        let _ = events::emit_scan_done(
            &app,
            &ScanDone {
                scan_id,
                module: empty_folders::MODULE_ID.to_owned(),
                finding_count: output.findings.len() as u64,
            },
        );

        Ok(EmptyFolderScanResult {
            scan_id,
            root,
            unreadable: output.unreadable.len() as u64,
            skipped_protected: output.skipped_protected,
            folders_visited: output.folders_visited,
            cancelled: output.cancelled,
            findings: output.findings,
            recommendations,
        })
    })
    .await
}

fn percent(done: u64, total: u64) -> Option<u8> {
    if total == 0 {
        return None;
    }
    u8::try_from(done.saturating_mul(100) / total).ok()
}

/* -------------------------------------------------------------------------
 * Treemap: scan a drive once, then serve one directory at a time
 * ---------------------------------------------------------------------- */

/// What a treemap scan hands the UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreemapScanResult {
    /// Id for `cancel_scan`.
    pub scan_id: Uuid,
    /// The header row that was cached, so the UI can show staleness at once.
    pub summary: treemap::CachedScan,
    /// Reparse points that were drawn but deliberately not counted.
    pub links: Vec<PathBuf>,
    /// Directories the walk could not list.
    pub unreadable: Vec<PathBuf>,
    /// Whether the user stopped it.
    pub cancelled: bool,
    /// The root level, so the first paint needs no second round trip.
    pub level: treemap::TreeLevel,
}

/// Drives the treemap can map, with their capacity.
///
/// # Errors
/// Infallible today; keeps the shared `Result` shape.
#[tauri::command(async)]
pub fn treemap_drives() -> Result<Vec<crate::core::platform::DriveInfo>, ApiError> {
    guarded("treemap_drives", || {
        use crate::core::platform::Volumes;
        Ok(crate::core::platform::SystemVolumes.drives())
    })
}

/// Walk `root`, aggregate it, cache it, and return its top level.
///
/// Read-only apart from the app's own cache. Blocking work; the caller runs it
/// off the UI thread.
///
/// # Errors
/// Returns an error if `root` is not a mappable local path, if the cache cannot
/// be written, or if the root cannot be served.
#[tauri::command]
pub async fn treemap_scan<R: Runtime>(
    app: AppHandle<R>,
    root: PathBuf,
    db: State<'_, Arc<Database>>,
    cancels: State<'_, CancelRegistry>,
) -> Result<TreemapScanResult, ApiError> {
    guarded_async("treemap_scan", async {
        // The root arrives from the WebView unchecked, exactly as `treemap_children`'s
        // did (security review M17).
        let root = mappable_root(&root)?;
        let scan_id = Uuid::new_v4();
        let started_at = Utc::now();
        let cancel = cancels.start(scan_id);
        let mut throttle = ProgressThrottle::new();

        let scan = {
            let mut emit = |progress: &treemap::TreeProgress<'_>| {
                if !throttle.should_emit() {
                    return;
                }
                let payload = ScanProgress {
                    scan_id,
                    module: "treemap".to_owned(),
                    items_scanned: progress.entries,
                    current_path: Some(progress.current_path.display().to_string()),
                    pct: None,
                    items_total: None,
                };
                if let Err(err) = events::emit_scan_progress(&app, &payload) {
                    tracing::debug!(error = %err, "treemap progress event was not delivered");
                }
            };
            treemap::scan(&root, &cancel, Some(&mut emit))
        };
        cancels.finish(scan_id);

        tracing::info!(
            scan = %scan_id,
            root = %root.display(),
            dirs = scan.dirs.len(),
            bytes = scan.root_total().map_or(0, |t| t.total_bytes),
            cancelled = scan.cancelled,
            "treemap scan finished"
        );

        // A cancelled walk is still cached: a partial picture beats no picture, and
        // `complete: false` stops anyone reading it as a total.
        let summary = db.with(|conn| {
            treemap::save_cache(conn, &scan)?;
            treemap::cached_scan(conn, &root)
        })?;
        let summary = summary.ok_or_else(|| {
            CoreError::Quarantine(
                "the treemap cache was written but could not be read back".to_owned(),
            )
        })?;

        remember(
            &db,
            scan_id,
            "treemap",
            started_at,
            summary.file_count,
            0,
            scan.cancelled,
        );

        let oracle = treemap::MemoryOracle::new(&scan);
        let level = treemap::children(&root, &root, &oracle)?;

        let _ = events::emit_scan_done(
            &app,
            &ScanDone {
                scan_id,
                module: "treemap".to_owned(),
                finding_count: scan.dirs.len() as u64,
            },
        );

        Ok(TreemapScanResult {
            scan_id,
            summary,
            links: scan.links,
            unreadable: scan.unreadable,
            cancelled: scan.cancelled,
            level,
        })
    })
    .await
}

/// One directory's children, sized from the cached scan of `root`.
///
/// This is the zoom step. It reads a single directory from disk, so file names
/// are always current, and takes subdirectory totals from the cache.
///
/// # Errors
/// Returns an error if `root` is not a mappable local path, if `path` is not
/// inside it, or if the cache cannot be read.
#[tauri::command(async)]
pub fn treemap_children(
    root: PathBuf,
    path: PathBuf,
    db: State<'_, Arc<Database>>,
) -> Result<treemap::TreeLevel, ApiError> {
    guarded("treemap_children", || {
        let (root, path) = mappable_level(&root, &path)?;
        let oracle = treemap::CacheOracle::new(&**db, &root);
        Ok(treemap::children(&path, &root, &oracle)?)
    })
}

/// The gate every treemap root crosses.
///
/// The same chokepoint the cleaner's `resolve_root` uses: a UNC share, a device
/// path, an NT-namespace spelling or an 8.3 alias aimed somewhere else is
/// refused rather than mapped, and what comes back is the resolved form, so a
/// containment check compares the place rather than the spelling
/// (security review M17).
///
/// The treemap is read-only, so this is not about protecting a location — it is
/// about not enumerating one the user did not name, and about the nine spellings
/// of a path being refused here as they are everywhere else.
fn mappable_root(root: &Path) -> Result<PathBuf, ApiError> {
    match crate::core::brain::gate_path(root) {
        crate::core::brain::GateVerdict::Local(canonical) => Ok(canonical),
        crate::core::brain::GateVerdict::Missing => Err(CoreError::Quarantine(format!(
            "`{}` is not on this machine, so there is nothing to map",
            root.display()
        ))
        .into()),
        crate::core::brain::GateVerdict::Refused(why) => Err(CoreError::Quarantine(format!(
            "`{}` cannot be mapped because {why}",
            root.display()
        ))
        .into()),
    }
}

/// Both halves of a zoom request, gated and tied together.
///
/// `root` and `path` both arrive from the WebView, and nothing used to tie them
/// together: `children(path = C:\Windows\System32\drivers\etc, root = <a
/// tempdir>)` enumerated `hosts`, `services`, `lmhosts.sam`, and `breadcrumb`
/// then walked up from `path` all the way to `C:\`, handing back the whole
/// ancestor chain. Read-only, so no hard rule broke — but every other
/// path-taking command in this file goes through `gate_path` or `path_compare`,
/// and this was the exception (security review M17).
///
/// # Errors
/// Returns an error when `root` is not a plain local path that exists, or when
/// `path` is not inside it.
fn mappable_level(root: &Path, path: &Path) -> Result<(PathBuf, PathBuf), ApiError> {
    let root = mappable_root(root)?;
    if !crate::core::path_compare::is_inside(&root, path) {
        return Err(CoreError::Quarantine(format!(
            "`{}` is not inside `{}`, so it is not part of that map",
            path.display(),
            root.display()
        ))
        .into());
    }
    Ok((root, path.to_path_buf()))
}

/// The cached header for a root, or `None` when it has never been scanned.
///
/// # Errors
/// Returns an error if the cache cannot be read.
#[tauri::command(async)]
pub fn treemap_cached(
    root: PathBuf,
    db: State<'_, Arc<Database>>,
) -> Result<Option<treemap::CachedScan>, ApiError> {
    guarded("treemap_cached", || {
        Ok(db.with(|conn| treemap::cached_scan(conn, &root))?)
    })
}

/// Open a path in Explorer, selecting it when it is a file.
///
/// The Swiss-knife glue module spec 04 asks for. It shows a location; it never
/// changes one.
///
/// # Errors
/// Returns an error if Explorer could not be started.
#[tauri::command]
pub fn reveal_in_explorer(path: PathBuf) -> Result<(), ApiError> {
    guarded("reveal_in_explorer", || {
        crate::core::platform::reveal_in_explorer(&path).map_err(Into::into)
    })
}

/* -------------------------------------------------------------------------
 * Startup manager
 * ---------------------------------------------------------------------- */

/// What a startup scan hands the UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupScanResult {
    /// Id `execute` needs when a toggle turns something off.
    pub scan_id: Uuid,
    /// The entries, sorted by source then name.
    pub scan: startup::StartupScan,
    /// One per entry: the recommendation a toggle applies, so a disable goes
    /// through the executor's four gates and lands in the audit log like every
    /// other change (module spec 05).
    pub recommendations: Vec<crate::core::types::Recommendation>,
}

/// Enumerate everything that runs at logon. Read-only.
///
/// # Errors
/// Returns an error only if the ledger of disabled shortcuts cannot be read.
#[tauri::command]
pub async fn startup_scan<R: Runtime>(
    app: AppHandle<R>,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
    db: State<'_, Arc<Database>>,
) -> Result<StartupScanResult, ApiError> {
    guarded_async("startup_scan", async {
        let started_at = Utc::now();
        let scan = services.startup_service.scan(&**db);
        let scan_id = Uuid::new_v4();

        let mut findings = Vec::with_capacity(scan.entries.len());
        let mut recommendations = Vec::with_capacity(scan.entries.len());
        for entry in &scan.entries {
            let (finding, recommendation) = startup_recommendation(entry, &services.brain);
            findings.push(finding);
            if let Some(recommendation) = recommendation {
                recommendations.push(recommendation);
            }
        }

        store.insert(ScanResult {
            scan_id,
            module: "startup".to_owned(),
            findings,
            recommendations: recommendations.clone(),
            skipped_rules: scan.unreadable.clone(),
            cancelled: false,
        });

        remember(
            &db,
            scan_id,
            "startup",
            started_at,
            scan.entries.len() as u64,
            0,
            false,
        );

        tracing::info!(
            scan = %scan_id,
            entries = scan.entries.len(),
            unreadable = scan.unreadable.len(),
            "startup scan finished"
        );
        let _ = events::emit_scan_done(
            &app,
            &ScanDone {
                scan_id,
                module: "startup".to_owned(),
                finding_count: scan.entries.len() as u64,
            },
        );

        Ok(StartupScanResult {
            scan_id,
            scan,
            recommendations,
        })
    })
    .await
}

/* ----------------------------------------------------------------------
 * Tweaks
 * ---------------------------------------------------------------------- */

/// What a tweaks scan hands the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TweaksScanResult {
    /// Id `execute` needs to apply a tweak.
    pub scan_id: Uuid,
    /// Every catalogue tweak with its live value.
    pub tweaks: Vec<tweaks::TweakState>,
    /// One per tweak that can still be applied, so applying goes through the
    /// executor's gates and lands in the audit log.
    pub recommendations: Vec<crate::core::types::Recommendation>,
}

/// Read every catalogue tweak's live value. Read-only.
///
/// # Errors
/// Returns an error only if the undo records cannot be read.
#[tauri::command]
pub async fn tweaks_scan<R: Runtime>(
    app: AppHandle<R>,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
    db: State<'_, Arc<Database>>,
) -> Result<TweaksScanResult, ApiError> {
    guarded_async("tweaks_scan", async {
        let started_at = Utc::now();
        let states = db.with(|conn| services.tweaks.scan(conn))?;
        let scan_id = Uuid::new_v4();
        let (findings, recommendations): (Vec<_>, Vec<_>) =
            states.iter().filter_map(tweaks::recommendation).unzip();

        store.insert(ScanResult {
            scan_id,
            module: "tweaks".to_owned(),
            findings,
            recommendations: recommendations.clone(),
            skipped_rules: Vec::new(),
            cancelled: false,
        });
        remember(
            &db,
            scan_id,
            "tweaks",
            started_at,
            states.len() as u64,
            0,
            false,
        );
        let _ = events::emit_scan_done(
            &app,
            &ScanDone {
                scan_id,
                module: "tweaks".to_owned(),
                finding_count: states.len() as u64,
            },
        );

        Ok(TweaksScanResult {
            scan_id,
            tweaks: states,
            recommendations,
        })
    })
    .await
}

/// Put a tweak's recorded prior value back.
///
/// # Errors
/// Returns an error when there is nothing to undo, when the value was changed
/// since the tweak was applied, or when the write or read-back fails.
#[tauri::command(async)]
pub fn tweaks_undo(
    tweak_id: String,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<(), ApiError> {
    guarded("tweaks_undo", || {
        Ok(db.with(|conn| services.tweaks.undo(conn, &tweak_id))?)
    })
}

/// What a context-menu scan hands the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextMenuScanResult {
    /// Id `execute` needs to disable an entry.
    pub scan_id: Uuid,
    /// Every verb and shell-extension handler found, in both hives.
    pub entries: Vec<context_menu::ContextMenuEntry>,
    /// One per per-user entry that can still be disabled.
    pub recommendations: Vec<crate::core::types::Recommendation>,
}

/// Read every context-menu entry. Read-only.
///
/// # Errors
/// Returns an error only if the undo records cannot be read.
#[tauri::command]
pub async fn context_menu_scan<R: Runtime>(
    app: AppHandle<R>,
    services: State<'_, Services>,
    store: State<'_, ScanStore>,
    db: State<'_, Arc<Database>>,
) -> Result<ContextMenuScanResult, ApiError> {
    guarded_async("context_menu_scan", async {
        let started_at = Utc::now();
        let entries = db.with(|conn| services.context_menu.scan(conn))?;
        let scan_id = Uuid::new_v4();
        let (findings, recommendations): (Vec<_>, Vec<_>) = entries
            .iter()
            .filter_map(context_menu::recommendation)
            .unzip();

        store.insert(ScanResult {
            scan_id,
            module: "context-menu".to_owned(),
            findings,
            recommendations: recommendations.clone(),
            skipped_rules: Vec::new(),
            cancelled: false,
        });
        remember(
            &db,
            scan_id,
            "context-menu",
            started_at,
            entries.len() as u64,
            0,
            false,
        );
        let _ = events::emit_scan_done(
            &app,
            &ScanDone {
                scan_id,
                module: "context-menu".to_owned(),
                finding_count: entries.len() as u64,
            },
        );

        Ok(ContextMenuScanResult {
            scan_id,
            entries,
            recommendations,
        })
    })
    .await
}

/// Re-enable a context-menu entry WinKnife disabled.
///
/// # Errors
/// Returns an error when there is nothing to undo, when the value was changed
/// since, or when the write or read-back fails.
#[tauri::command(async)]
pub fn context_menu_undo(
    entry_id: String,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<(), ApiError> {
    guarded("context_menu_undo", || {
        Ok(db.with(|conn| services.context_menu.undo(conn, &entry_id))?)
    })
}

/// How every note explaining a withheld green rating opens.
const STARTUP_NOT_RATED_SAFE: &str = "It is not rated safe:";

/// Said when the program a safe-to-disable entry runs could not be found, so
/// the protected-path gate has nothing to judge.
const STARTUP_UNRESOLVED_PROGRAM: &str = "the program it runs could not be worked out from its \
     command line, so WinKnife cannot check that it lives outside a protected location.";

/// Why an entry the known-app list calls safe to disable still may not be rated
/// green, or `None` when it may. This is the cleaner's gate
/// (`Brain::path_refusal`), so "protected" means one thing everywhere and no
/// startup entry whose program lives in `C:\Windows` or `C:\Program Files*`
/// comes out Safe (CLAUDE.md hard rule).
fn startup_safe_refusal(entry: &startup::StartupEntry, brain: &Brain) -> Option<String> {
    let Some(program) = entry.target.as_deref() else {
        return Some(STARTUP_UNRESOLVED_PROGRAM.to_owned());
    };
    let refusal = brain.path_refusal(program)?;
    // Set only when the spelling hid the place (an 8.3 alias, a junction).
    let named = match &refusal.canonical {
        Some(canonical) => format!("`{}` (really `{}`)", program.display(), canonical.display()),
        None => format!("`{}`", program.display()),
    };
    // `refusal.reason` is worded for the cleaner ("not cleanable … use the
    // app's own uninstaller"). The list's own entry, up to its advice, names
    // the place; a gate refusal the list did not make keeps the gate's words.
    let judged = refusal.canonical.as_deref().unwrap_or(program);
    let place = brain.protected_paths().reason_for(judged).map_or_else(
        || refusal.reason.clone(),
        |listed| {
            listed
                .split_once(": ")
                .map_or(listed, |(place, _)| place)
                .to_owned()
        },
    );
    Some(format!(
        "the program it runs, {named}, is in a place WinKnife protects ({place}), and a \
         startup entry is only rated safe when its program lives outside every such place."
    ))
}

/// The bucket and the reason for an enabled third-party entry. The known-app
/// list only nominates an entry for green; the program it runs still has to
/// clear the protected-path gate before it gets there.
fn startup_rating(
    entry: &startup::StartupEntry,
    brain: &Brain,
) -> (crate::core::types::RiskBucket, String) {
    use crate::core::types::RiskBucket;

    let withheld = match entry.verdict {
        startup::Verdict::SafeToDisable => startup_safe_refusal(entry, brain),
        _ => None,
    };
    let reason = format!(
        "{} Turning this off stops it starting at logon; it can be turned back on here at \
         any time, and nothing is removed. Estimated impact: {:?}. {}",
        entry.note, entry.impact, entry.impact_reason
    );
    match (entry.verdict, withheld) {
        (startup::Verdict::SafeToDisable, None) => (RiskBucket::Safe, reason),
        (_, None) => (RiskBucket::NeedsReview, reason),
        (_, Some(withheld)) => (
            RiskBucket::NeedsReview,
            format!("{reason} {STARTUP_NOT_RATED_SAFE} {withheld}"),
        ),
    }
}

/// Build the finding and, when the entry can be toggled, the recommendation the
/// executor will re-verify.
fn startup_recommendation(
    entry: &startup::StartupEntry,
    brain: &Brain,
) -> (
    crate::core::types::Finding,
    Option<crate::core::types::Recommendation>,
) {
    use crate::core::types::{Action, Finding, FindingKind, Justification};

    let kind = match entry.source {
        crate::core::types::StartupSource::ScheduledTask => FindingKind::ScheduledTask,
        crate::core::types::StartupSource::UserStartupFolder
        | crate::core::types::StartupSource::CommonStartupFolder => FindingKind::File,
        _ => FindingKind::RegistryEntry,
    };
    let mut finding = Finding::new("startup", PathBuf::from(&entry.command), kind);
    finding.meta = serde_json::json!({
        "entryId": entry.id,
        "source": entry.source,
        "key": entry.key,
    });

    // Windows' own entries and anything already off get a finding but no
    // action — the same shape the cleaner's red bucket uses.
    if entry.os_owned || !entry.enabled {
        return (finding, None);
    }

    let (risk, reason) = startup_rating(entry, brain);
    let recommendation = crate::core::types::Recommendation::new(
        vec![finding.id],
        risk,
        if entry.signed { 80 } else { 55 },
        Action::DisableStartupEntry,
        reason,
        // Disabling an autostart entry frees no disk space, and claiming a
        // figure here would put an invented number in the report.
        0,
        Justification::StartupEntry {
            source: entry.source,
            key: entry.key.clone(),
            command: entry.command.clone(),
        },
    );

    match recommendation {
        Ok(recommendation) => (finding, Some(recommendation)),
        Err(err) => {
            tracing::error!(entry = entry.id, error = %err, "dropped a startup recommendation");
            (finding, None)
        }
    }
}

/// Turn an autostart entry back on.
///
/// The reversal, and deliberately not an executor action: there is no
/// recommendation to re-verify when putting something back the way the user
/// found it, exactly as restoring from quarantine is not an executor action
/// either. Turning something **off** goes through `execute`.
///
/// Returns what the user should know about a shortcut that came back only in
/// part (round-2 review); empty otherwise.
///
/// # Errors
/// Returns an error when the entry is unknown, when Windows owns it, or when
/// the change was refused (an `HKLM` entry needs elevation).
#[tauri::command(async)]
pub fn startup_enable(
    source: crate::core::types::StartupSource,
    key: String,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<Vec<crate::core::types::MoveWarning>, ApiError> {
    guarded("startup_enable", || {
        // Re-enabling moves a held shortcut back out of the drawer, so it is
        // refused while a clean, restore or purge is running.
        let _busy = services.quarantine.begin(Operation::Restore)?;
        Ok(db.with(|conn| {
            services
                .startup_service
                .set_enabled(conn, source, &key, true, Utc::now())
        })?)
    })
}

/// Write one line of scan history, logging rather than failing.
///
/// A dashboard card that cannot be written must never fail the scan that
/// produced it — the user asked for a scan, not for a card.
fn remember(
    db: &Database,
    scan_id: Uuid,
    module: &str,
    started_at: chrono::DateTime<Utc>,
    finding_count: u64,
    reclaimable_bytes: u64,
    cancelled: bool,
) {
    if let Err(err) = db.with(|conn| {
        history::record(
            conn,
            scan_id,
            module,
            started_at,
            finding_count,
            reclaimable_bytes,
            cancelled,
        )
    }) {
        tracing::warn!(module, error = %err, "scan history was not written");
    }
}

/* -------------------------------------------------------------------------
 * Dashboard
 * ---------------------------------------------------------------------- */

/// One module's card.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleCard {
    /// Module id, so the UI can join it to the manifest it already has.
    pub module: String,
    /// The last run, or `None` when it has never been run on this machine.
    pub last_scan: Option<history::ScanRecord>,
}

/// Everything the dashboard shows, in one round trip.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummary {
    /// One per registered module, in sidebar order.
    pub modules: Vec<ModuleCard>,
    /// What is currently held, and how many ledger rows no longer parse.
    pub quarantine: QuarantineStats,
    /// The armed timer, if any.
    pub timer: TimerStatus,
    /// Drives, so the dashboard can show how full the machine is.
    pub drives: Vec<crate::core::platform::DriveInfo>,
}

/// Everything the dashboard shows.
///
/// # Errors
/// Returns an error if the history or the quarantine ledger cannot be read.
#[tauri::command(async)]
pub fn dashboard_summary(
    registry: State<'_, ModuleRegistry>,
    services: State<'_, Services>,
    timer: State<'_, Arc<ShutdownTimer>>,
    db: State<'_, Arc<Database>>,
) -> Result<DashboardSummary, ApiError> {
    guarded("dashboard_summary", || {
        use crate::core::platform::Volumes;

        let (modules, quarantine) = db.with(|conn| {
            let mut cards = Vec::with_capacity(registry.len());
            for manifest in registry.manifests() {
                cards.push(ModuleCard {
                    last_scan: history::latest(conn, &manifest.id)?,
                    module: manifest.id,
                });
            }
            Ok((cards, services.quarantine.stats(conn)?))
        })?;

        Ok(DashboardSummary {
            modules,
            quarantine,
            timer: timer.status(Utc::now()),
            drives: crate::core::platform::SystemVolumes.drives(),
        })
    })
}

/* -------------------------------------------------------------------------
 * Settings
 * ---------------------------------------------------------------------- */

/// Everything the settings page shows.
///
/// # Errors
/// Returns an error if the settings table cannot be read.
#[tauri::command(async)]
pub fn get_settings(
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<AppSettings, ApiError> {
    guarded("get_settings", || {
        Ok(db.with(|conn| read_settings(&services, conn))?)
    })
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
#[tauri::command(async)]
pub fn set_settings(
    settings_in: AppSettings,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<AppSettings, ApiError> {
    guarded("set_settings", || {
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
    })
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
        .filter(|pack| pack.spec.module == cleaner::CLEANER_MODULE_ID)
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
#[tauri::command(async)]
pub fn quarantine_list(
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<Vec<QuarantineBatch>, ApiError> {
    guarded("quarantine_list", || {
        Ok(db.with(|conn| services.quarantine.list_batches(conn))?)
    })
}

/// Items of one batch, so the drawer can expand it.
///
/// # Errors
/// Returns an error if the ledger cannot be read.
#[tauri::command(async)]
pub fn quarantine_batch_items(
    batch_id: Uuid,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<Vec<QuarantineItem>, ApiError> {
    guarded("quarantine_batch_items", || {
        Ok(db.with(|conn| services.quarantine.items_in_batch(conn, batch_id))?)
    })
}

/// Totals across everything still held.
///
/// # Errors
/// Returns an error if the ledger cannot be read.
#[tauri::command(async)]
pub fn quarantine_stats(
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<QuarantineStats, ApiError> {
    guarded("quarantine_stats", || {
        Ok(db.with(|conn| services.quarantine.stats(conn))?)
    })
}

/// What a restore did, per item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    /// Items that went back.
    pub restored: Vec<RestoreOutcome>,
    /// Items that did not, with the reason.
    pub failed: Vec<RestoreFailure>,
    /// The user pressed Stop; the rest of the batch is still in quarantine.
    #[serde(default)]
    pub cancelled: bool,
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
/// Reports progress on [`events::RESTORE_PROGRESS`] and stops between items
/// when `cancel_scan` is called with the batch id (issue #39).
///
/// # Errors
/// Returns an error only if the ledger cannot be read; per-item failures are
/// in the report.
#[tauri::command(async)]
pub fn quarantine_restore<R: Runtime>(
    app: AppHandle<R>,
    batch_id: Uuid,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
    cancels: State<'_, CancelRegistry>,
) -> Result<RestoreReport, ApiError> {
    guarded("quarantine_restore", || {
        let _busy = services.quarantine.begin(Operation::Restore)?;
        let ticket = cancels.track(batch_id);
        let mut throttle = ProgressThrottle::new();
        let mut emit = |done: u64, total: u64, path: &std::path::Path| {
            if done != total && !throttle.should_emit() {
                return;
            }
            let payload = events::RestoreProgress {
                batch_id,
                items_done: done,
                items_total: total,
                current_path: Some(path.display().to_string()),
            };
            if let Err(err) = events::emit_restore_progress(&app, &payload) {
                tracing::debug!(error = %err, "restore progress event was not delivered");
            }
        };
        let outcome = services.quarantine.restore_batch_controlled(
            &**db,
            batch_id,
            Some(ticket.token()),
            Some(&mut emit),
        )?;
        notify_quarantine_changed(&app, &services, &db, Some(batch_id));
        Ok(RestoreReport::from(outcome))
    })
}

impl From<crate::core::quarantine::BatchRestore> for RestoreReport {
    fn from(outcome: crate::core::quarantine::BatchRestore) -> Self {
        Self {
            restored: outcome.restored,
            failed: outcome
                .failed
                .into_iter()
                .map(|(item_id, error)| RestoreFailure { item_id, error })
                .collect(),
            cancelled: outcome.cancelled,
        }
    }
}

/// Restore one item.
///
/// # Errors
/// Returns [`CoreError::UnknownQuarantineItem`] for an unknown id.
#[tauri::command(async)]
pub fn quarantine_restore_item<R: Runtime>(
    app: AppHandle<R>,
    item_id: Uuid,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<RestoreOutcome, ApiError> {
    guarded("quarantine_restore_item", || {
        let _busy = services.quarantine.begin(Operation::Restore)?;
        let outcome = services.quarantine.restore_item(&**db, item_id)?;
        notify_quarantine_changed(&app, &services, &db, None);
        Ok(outcome)
    })
}

/// Run the retention purge and the size cap now.
///
/// # Errors
/// Returns an error if the ledger cannot be read.
#[tauri::command(async)]
pub fn quarantine_purge<R: Runtime>(
    app: AppHandle<R>,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<PurgeReport, ApiError> {
    guarded("quarantine_purge", || {
        let _busy = services.quarantine.begin(Operation::Purge)?;
        let report = services.quarantine.purge(&**db, Utc::now())?;
        notify_quarantine_changed(&app, &services, &db, None);
        Ok(report)
    })
}

/// Remove one item whose encrypted restore was refused, at the user's
/// explicit request from the drawer. The automatic purge never removes such
/// an item (decision b, 2026-10-12).
///
/// # Errors
/// Returns [`CoreError::UnknownQuarantineItem`] for an id that is not held,
/// and [`CoreError::Quarantine`] for an item whose restore was never refused.
#[tauri::command(async)]
pub fn quarantine_purge_item<R: Runtime>(
    app: AppHandle<R>,
    item_id: Uuid,
    services: State<'_, Services>,
    db: State<'_, Arc<Database>>,
) -> Result<PurgeReport, ApiError> {
    guarded("quarantine_purge_item", || {
        let _busy = services.quarantine.begin(Operation::Purge)?;
        let report = services.quarantine.purge_item(&**db, item_id)?;
        notify_quarantine_changed(&app, &services, &db, None);
        Ok(report)
    })
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
#[tauri::command(async)]
pub fn shutdown_schedule(
    mode: ScheduleMode,
    action: PowerAction,
    timer: State<'_, Arc<ShutdownTimer>>,
    db: State<'_, Arc<Database>>,
) -> Result<TimerStatus, ApiError> {
    guarded("shutdown_schedule", || {
        let now = Utc::now();
        let persisted = ShutdownTimer::prepare(mode, action, now)?;
        db.with(|conn| {
            crate::core::settings::set_json(
                conn,
                crate::core::settings::KEY_ACTIVE_TIMER,
                &persisted,
            )
        })?;
        timer.arm(persisted);
        Ok(timer.status(now))
    })
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
#[tauri::command(async)]
pub fn shutdown_cancel(
    timer: State<'_, Arc<ShutdownTimer>>,
    db: State<'_, Arc<Database>>,
) -> Result<TimerStatus, ApiError> {
    guarded("shutdown_cancel", || {
        db.with(|conn| {
            crate::core::settings::clear(conn, crate::core::settings::KEY_ACTIVE_TIMER)
        })?;
        timer.cancel();
        Ok(timer.status(Utc::now()))
    })
}

/// Current timer state.
///
/// # Errors
/// Infallible today; keeps the shared `Result` shape.
#[tauri::command(async)]
pub fn shutdown_status(timer: State<'_, Arc<ShutdownTimer>>) -> Result<TimerStatus, ApiError> {
    guarded("shutdown_status", || Ok(timer.status(Utc::now())))
}

/* -------------------------------------------------------------------------
 * Launch: `--module <id>` (phase 5, task 1)
 * ---------------------------------------------------------------------- */

/// The route a launch asked for, if the UI has not opened it yet.
///
/// Called by the shell once on mount (the first launch's `--module`) and again
/// on every `launch:navigate` (a forwarded one). Taking it clears it, so a
/// route is opened once. Already validated against the registry: an unknown id
/// arrives here as the dashboard.
///
/// # Errors
/// Infallible today; keeps the shared `Result` shape.
#[tauri::command]
pub fn take_launch_route(
    pending: State<'_, crate::PendingLaunchRoute>,
) -> Result<Option<String>, ApiError> {
    guarded("take_launch_route", || Ok(pending.take()))
}

/// Whether System Protection looks enabled. Read-only; `unknown` is a valid
/// answer (module spec 09).
///
/// # Errors
/// Infallible today; an unreadable registry is reported as `unknown`.
#[tauri::command(async)]
pub fn restore_point_status(
    service: State<'_, Arc<RestorePointService>>,
) -> Result<RestorePointStatus, ApiError> {
    guarded("restore_point_status", || Ok(service.status()))
}

/// Ask Windows for a restore point through the elevated helper. Shows a UAC
/// prompt and waits up to five minutes, off the UI thread.
///
/// # Errors
/// Returns an error while another request is running, or when this build may
/// not reach the OS. Every Windows answer, failure included, is an `Ok`
/// outcome the page explains.
#[tauri::command]
pub async fn restore_point_create(
    service: State<'_, Arc<RestorePointService>>,
    description: String,
) -> Result<CreateOutcome, ApiError> {
    guarded_async("restore_point_create", async {
        let service = Arc::clone(&service);
        tauri::async_runtime::spawn_blocking(move || service.create(&description))
            .await
            .map_err(|err| CoreError::RestorePoint(format!("the request did not finish: {err}")))?
            .map_err(ApiError::from)
    })
    .await
}

/// Open Windows' System Protection settings. Only ever on a button click; it
/// changes nothing by itself.
///
/// # Errors
/// Returns an error when the window could not be opened.
#[tauri::command]
pub fn restore_point_open_settings(
    service: State<'_, Arc<RestorePointService>>,
) -> Result<(), ApiError> {
    guarded("restore_point_open_settings", || {
        Ok(service.open_protection_settings()?)
    })
}

/* -------------------------------------------------------------------------
 * Hosts file
 * ---------------------------------------------------------------------- */

/// The hosts file, decoded and parsed. Read-only (K-019).
///
/// # Errors
/// Returns an error when the file exists but cannot be read or decoded.
#[tauri::command(async)]
pub fn hosts_read(service: State<'_, Arc<HostsService>>) -> Result<HostsDocument, ApiError> {
    guarded("hosts_read", || Ok(service.read()?))
}

/// Parse, validate and diff an edited text against the file. Writes nothing.
///
/// # Errors
/// Returns an error when the current file cannot be read.
#[tauri::command(async)]
pub fn hosts_preview(
    service: State<'_, Arc<HostsService>>,
    text: String,
) -> Result<HostsPreview, ApiError> {
    guarded("hosts_preview", || Ok(service.preview(&text)?))
}

/// Back up the hosts file, then write `text` through the elevated helper. Shows
/// a UAC prompt and waits for it, off the UI thread.
///
/// # Errors
/// Returns an error for invalid content, a save already running, or a backup
/// that could not be taken; every helper answer is an `Ok` outcome.
#[tauri::command]
pub async fn hosts_save(
    service: State<'_, Arc<HostsService>>,
    text: String,
) -> Result<SaveOutcome, ApiError> {
    guarded_async("hosts_save", async {
        let service = Arc::clone(&service);
        tauri::async_runtime::spawn_blocking(move || service.save(&text))
            .await
            .map_err(|err| CoreError::Hosts(format!("the request did not finish: {err}")))?
            .map_err(ApiError::from)
    })
    .await
}

/// WinKnife's hosts backups, newest first.
///
/// # Errors
/// Returns an error when the backup folder cannot be listed.
#[tauri::command(async)]
pub fn hosts_backups(service: State<'_, Arc<HostsService>>) -> Result<Vec<HostsBackup>, ApiError> {
    guarded("hosts_backups", || Ok(service.backups()?))
}

/// Write one backup back over the hosts file, taking a backup of the current
/// file first. Same UAC flow as [`hosts_save`].
///
/// # Errors
/// Returns an error for an unknown backup name or a save already running.
#[tauri::command]
pub async fn hosts_restore(
    service: State<'_, Arc<HostsService>>,
    name: String,
) -> Result<SaveOutcome, ApiError> {
    guarded_async("hosts_restore", async {
        let service = Arc::clone(&service);
        tauri::async_runtime::spawn_blocking(move || service.restore(&name))
            .await
            .map_err(|err| CoreError::Hosts(format!("the request did not finish: {err}")))?
            .map_err(ApiError::from)
    })
    .await
}

/// Show one backup in Explorer.
///
/// # Errors
/// Returns an error for a name that is not one of WinKnife's backups.
#[tauri::command]
pub fn hosts_reveal_backup(
    service: State<'_, Arc<HostsService>>,
    name: String,
) -> Result<(), ApiError> {
    guarded("hosts_reveal_backup", || {
        let path = service.backup_path(&name)?;
        Ok(crate::core::platform::reveal_in_explorer(&path)?)
    })
}

/// Run `ipconfig /flushdns`. Only ever on a button click.
///
/// # Errors
/// Returns an error when `ipconfig` could not run or reported a failure.
#[tauri::command]
pub async fn hosts_flush_dns() -> Result<String, ApiError> {
    guarded_async("hosts_flush_dns", async {
        tauri::async_runtime::spawn_blocking(|| {
            crate::core::platform::flush_dns(&crate::core::platform::SystemEnv)
        })
        .await
        .map_err(|err| CoreError::Hosts(format!("the request did not finish: {err}")))?
        .map_err(ApiError::from)
    })
    .await
}

/* -------------------------------------------------------------------------
 * Legacy panels
 * ---------------------------------------------------------------------- */

/// The legacy-panels catalogue with each entry's availability. Read-only
/// (module spec 10).
///
/// # Errors
/// Infallible today.
#[tauri::command(async)]
pub fn legacy_panels_list(
    service: State<'_, Arc<LegacyPanelsService>>,
) -> Result<Vec<PanelInfo>, ApiError> {
    guarded("legacy_panels_list", || Ok(service.list()))
}

/// Open one of Windows' own windows by catalogue id. Only ever on a button
/// click; it changes nothing by itself.
///
/// # Errors
/// Returns an error for an unknown id, a window missing on this edition, or a
/// shell failure. Saying No at a UAC prompt is not an error.
#[tauri::command]
pub fn legacy_panels_open(
    service: State<'_, Arc<LegacyPanelsService>>,
    id: String,
) -> Result<(), ApiError> {
    guarded("legacy_panels_open", || Ok(service.open(&id)?))
}

/* -------------------------------------------------------------------------
 * Keep awake
 * ---------------------------------------------------------------------- */

/// Start keep-awake, replacing any running session.
///
/// # Errors
/// Returns [`CoreError::InvalidTimer`] for zero minutes or more than 24 hours.
#[tauri::command]
pub async fn keep_awake_start(
    minutes: u32,
    display: bool,
    engine: State<'_, Arc<KeepAwake>>,
) -> Result<KeepAwakeStatus, ApiError> {
    guarded_async("keep_awake_start", async {
        // Joins the previous worker thread, so keep it off the async runtime.
        let engine = Arc::clone(&engine);
        tauri::async_runtime::spawn_blocking(move || engine.start(minutes, display, Utc::now()))
            .await
            .map_err(|err| CoreError::PowerAction(format!("keep-awake task failed: {err}")))?
            .map_err(ApiError::from)
    })
    .await
}

/// Stop keep-awake. Stopping when nothing runs is not an error.
///
/// # Errors
/// Fails only if the background task cannot be joined.
#[tauri::command]
pub async fn keep_awake_stop(
    engine: State<'_, Arc<KeepAwake>>,
) -> Result<KeepAwakeStatus, ApiError> {
    guarded_async("keep_awake_stop", async {
        let engine = Arc::clone(&engine);
        tauri::async_runtime::spawn_blocking(move || engine.stop())
            .await
            .map_err(|err| CoreError::PowerAction(format!("keep-awake task failed: {err}")).into())
    })
    .await
}

/// Current keep-awake state.
///
/// # Errors
/// Infallible today; keeps the shared `Result` shape.
#[tauri::command(async)]
pub fn keep_awake_status(engine: State<'_, Arc<KeepAwake>>) -> Result<KeepAwakeStatus, ApiError> {
    guarded("keep_awake_status", || Ok(engine.status()))
}

/* -------------------------------------------------------------------------
 * File unlocker
 * ---------------------------------------------------------------------- */

fn unlocker_error(err: crate::core::platform::RmError) -> ApiError {
    let code = match err {
        crate::core::platform::RmError::NotArmed => "not_armed",
        _ => "unlocker_failed",
    };
    ApiError::new(code, err.to_string())
}

/// Which programs have these files (or files inside these folders) open.
/// Read-only (module spec 07).
///
/// # Errors
/// Returns an error when the Restart Manager fails.
#[tauri::command]
pub async fn unlocker_query(
    paths: Vec<PathBuf>,
) -> Result<crate::modules::file_unlocker::UnlockerReport, ApiError> {
    guarded_async("unlocker_query", async {
        tauri::async_runtime::spawn_blocking(move || {
            let rm = crate::core::platform::real_restart_manager();
            crate::modules::file_unlocker::query(rm.as_ref(), &paths).map_err(unlocker_error)
        })
        .await
        .map_err(|err| {
            ApiError::new("unlocker_failed", "the query did not finish")
                .with_detail(err.to_string())
        })?
    })
    .await
}

/// Ask one window or console program to close, politely (`RmShutdown` with no
/// force flag). `start_time` is the decimal text a query returned; a mismatch
/// with the running process is refused.
///
/// # Errors
/// Returns an error for a malformed start time, when the Restart Manager
/// fails, or when this build may not reach the OS.
#[tauri::command]
pub async fn unlocker_request_close(
    pid: u32,
    start_time: String,
) -> Result<crate::modules::file_unlocker::CloseReport, ApiError> {
    guarded_async("unlocker_request_close", async {
        let start_time: u64 = start_time.parse().map_err(|_| {
            ApiError::new(
                "invalid_start_time",
                "the process start time is not a number",
            )
        })?;
        let process = crate::core::platform::UniqueProcess { pid, start_time };
        tauri::async_runtime::spawn_blocking(move || {
            let rm = crate::core::platform::real_restart_manager();
            crate::modules::file_unlocker::request_close(rm.as_ref(), process)
                .map_err(unlocker_error)
        })
        .await
        .map_err(|err| {
            ApiError::new("unlocker_failed", "the request did not finish")
                .with_detail(err.to_string())
        })?
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::brain::ProtectedPaths;
    use crate::core::platform::{EnvResolver, SystemEnv};
    use crate::core::types::{Action, RiskBucket, StartupSource};
    use crate::modules;

    /* --- the treemap zoom gate (security review M17) -------------------- */

    #[test]
    fn a_zoom_inside_the_mapped_root_is_allowed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let inside = dir.path().join("sub").join("deeper");
        std::fs::create_dir_all(&inside).expect("dirs");

        let (root, path) = mappable_level(dir.path(), &inside).expect("a zoom inside the root");
        assert!(crate::core::path_compare::is_inside(&root, &path));
    }

    /// The reproduction: a `path` the caller never mapped, under a `root` it
    /// did. It used to enumerate `hosts`, `services`, `lmhosts.sam` and hand
    /// back a breadcrumb all the way up to `C:\`.
    #[test]
    fn a_zoom_outside_the_mapped_root_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let elsewhere = Path::new(r"C:\Windows\System32\drivers\etc");

        let refusal = mappable_level(dir.path(), elsewhere)
            .expect_err("a path outside the mapped root must be refused");
        let message = format!("{refusal:?}");
        println!(
            "[M17] children(path={}, root={})",
            elsewhere.display(),
            dir.path().display()
        );
        println!("[M17] -> {message}");
        assert!(
            message.contains("is not inside"),
            "the refusal has to say why: {message}"
        );
    }

    /// A parent of the root is outside it just as much as a sibling is — that
    /// is the direction the breadcrumb leaked in.
    #[test]
    fn a_zoom_to_an_ancestor_of_the_mapped_root_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("mapped");
        std::fs::create_dir_all(&root).expect("dirs");

        assert!(
            mappable_level(&root, dir.path()).is_err(),
            "zooming out past the root is leaving the map"
        );
    }

    /// The nine spellings are refused here as everywhere else. A UNC path is
    /// not a plain local drive-letter path, whatever it resolves to.
    #[test]
    fn a_root_that_is_not_a_plain_local_path_is_refused() {
        for spelling in [
            r"\\localhost\C$\Windows",
            r"\\.\C:\Windows",
            r"\??\C:\Windows",
            r"\\server\share\folder",
            r"C:Windows",
            r"C:\Windows\..\Windows",
        ] {
            let verdict = mappable_root(Path::new(spelling));
            assert!(
                verdict.is_err(),
                "`{spelling}` was accepted as `{:?}`",
                verdict.ok()
            );
        }
    }

    /// The verbatim prefix is not one of the refused spellings — it names the
    /// same local path, and the gate's job is to hand back the resolved form so
    /// the containment check compares places rather than spellings. Pinned so a
    /// future tightening does not quietly turn it into a refusal that a user's
    /// long path would trip over.
    #[test]
    fn a_verbatim_root_is_normalised_rather_than_refused() {
        let windows = Path::new(r"C:\Windows");
        if !windows.is_dir() {
            return;
        }
        assert_eq!(
            mappable_root(Path::new(r"\\?\C:\Windows")).expect("verbatim resolves"),
            mappable_root(windows).expect("plain resolves"),
        );
    }

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
            cancelled: true,
        })
        .expect("serialize");
        assert_eq!(
            json.get("cancelled"),
            Some(&serde_json::json!(true)),
            "got {json}"
        );
        assert!(
            json.get("failed")
                .and_then(|f| f.get(0))
                .and_then(|f| f.get("itemId"))
                .is_some(),
            "got {json}"
        );
    }

    /* --- startup ratings pass the protected-path gate (CLAUDE.md) ------- */

    /// The gate the app itself builds (`lib.rs`), from the real environment,
    /// with its own storage pointed into a scratch folder.
    fn app_brain(scratch: &Path) -> Brain {
        Brain::builder()
            .protected_paths(ProtectedPaths::from_env(
                &SystemEnv,
                &scratch.join("app-data"),
                &scratch.join("quarantine"),
            ))
            .build()
    }

    /// An enabled, signed, third-party `Run` entry.
    fn startup_entry(target: Option<PathBuf>, verdict: startup::Verdict) -> startup::StartupEntry {
        let command = target.as_deref().map_or_else(
            || "launcher --quiet".to_owned(),
            |program| format!("\"{}\" --quiet", program.display()),
        );
        startup::StartupEntry {
            id: "hkcu-run:Launcher".to_owned(),
            name: "Launcher".to_owned(),
            source: StartupSource::CurrentUserRun,
            key: "Launcher".to_owned(),
            location: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run".to_owned(),
            command,
            target,
            enabled: true,
            os_owned: false,
            os_reason: None,
            publisher: Some("Example Ltd".to_owned()),
            signed: true,
            signature_note: "Signed by Example Ltd".to_owned(),
            impact: startup::Impact::High,
            impact_reason: "Estimated from WinKnife's known-app list for this program.".to_owned(),
            verdict,
            category: Some("Games".to_owned()),
            note: "Starts itself when you open it.".to_owned(),
            leftover_shortcut: None,
        }
    }

    /// A real program file somewhere the gate clears.
    fn harmless_program(scratch: &Path) -> PathBuf {
        let folder = scratch.join("programs");
        std::fs::create_dir_all(&folder).expect("dirs");
        let program = folder.join("launcher.exe");
        std::fs::write(&program, b"MZ").expect("write");
        program
    }

    /// A location that exists under `%ProgramFiles%` on every Windows install.
    fn existing_program_files_location() -> PathBuf {
        let root = PathBuf::from(
            SystemEnv
                .var("ProgramFiles")
                .unwrap_or_else(|| r"C:\Program Files".to_owned()),
        );
        let common = root.join("Common Files");
        let location = if common.is_dir() { common } else { root };
        assert!(
            location.is_dir(),
            "`{}` has to exist for this test to mean anything",
            location.display()
        );
        location
    }

    /// The reproduction: `startup-known.json` calls anything whose command
    /// contains "Steam" safe to disable, and `C:\Program Files (x86)\Steam`
    /// used to come out green. A protected program is never rated Safe, and
    /// the reason is the gate's own — not "the file is missing".
    #[test]
    fn a_safe_to_disable_entry_whose_program_is_protected_is_not_rated_safe() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let brain = app_brain(scratch.path());
        let program = existing_program_files_location();
        let refusal = brain
            .path_refusal(&program)
            .expect("the cleaner's gate refuses %ProgramFiles%");

        let entry = startup_entry(Some(program.clone()), startup::Verdict::SafeToDisable);
        let (_, recommendation) = startup_recommendation(&entry, &brain);
        let recommendation = recommendation.expect("an enabled entry gets a recommendation");

        assert_eq!(recommendation.risk, RiskBucket::NeedsReview);
        assert!(
            recommendation
                .reason
                .contains("an installed-programs directory"),
            "the gate's verdict has to reach the user: {}",
            recommendation.reason
        );
        // The gate words its refusal for the cleaner; none of that reads right
        // on the Startup page, where nothing is cleaned or uninstalled.
        for cleaner_wording in ["not cleanable", "uninstaller", &refusal.reason] {
            assert!(
                !recommendation.reason.contains(cleaner_wording),
                "`{cleaner_wording}` is the cleaner's wording: {}",
                recommendation.reason
            );
        }
        assert!(
            recommendation
                .reason
                .contains(&program.display().to_string()),
            "the reason names the protected program: {}",
            recommendation.reason
        );
        assert!(
            !recommendation.reason.contains(STARTUP_UNRESOLVED_PROGRAM),
            "the program was found; it is protected, not missing: {}",
            recommendation.reason
        );
        assert!(
            recommendation.reason.starts_with(&entry.note),
            "the known-app note still leads: {}",
            recommendation.reason
        );
    }

    #[test]
    fn a_safe_to_disable_entry_whose_program_the_gate_clears_is_rated_safe() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let brain = app_brain(scratch.path());
        let program = harmless_program(scratch.path());
        assert!(
            brain.path_refusal(&program).is_none(),
            "a tempdir program is not protected"
        );

        let entry = startup_entry(Some(program), startup::Verdict::SafeToDisable);
        let (finding, recommendation) = startup_recommendation(&entry, &brain);
        let recommendation = recommendation.expect("an enabled entry gets a recommendation");

        assert_eq!(recommendation.risk, RiskBucket::Safe);
        assert_eq!(recommendation.confidence, 80, "signed entries keep 80");
        assert_eq!(recommendation.action, Action::DisableStartupEntry);
        assert_eq!(recommendation.finding_ids, vec![finding.id]);
        assert!(
            !recommendation.reason.contains(STARTUP_NOT_RATED_SAFE),
            "nothing to explain when it is green: {}",
            recommendation.reason
        );
    }

    #[test]
    fn a_safe_to_disable_entry_whose_program_is_unknown_is_not_rated_safe() {
        let brain = app_brain(tempfile::tempdir().expect("tempdir").path());
        let entry = startup_entry(None, startup::Verdict::SafeToDisable);

        let (_, recommendation) = startup_recommendation(&entry, &brain);
        let recommendation = recommendation.expect("an enabled entry gets a recommendation");

        assert_eq!(recommendation.risk, RiskBucket::NeedsReview);
        assert!(
            recommendation.reason.contains(STARTUP_UNRESOLVED_PROGRAM),
            "the reason says the program could not be worked out: {}",
            recommendation.reason
        );
    }

    #[test]
    fn other_startup_verdicts_stay_needs_review_without_a_gate_note() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let brain = app_brain(scratch.path());
        let program = harmless_program(scratch.path());

        for verdict in [
            startup::Verdict::Keep,
            startup::Verdict::UserChoice,
            startup::Verdict::Unknown,
        ] {
            let entry = startup_entry(Some(program.clone()), verdict);
            let (_, recommendation) = startup_recommendation(&entry, &brain);
            let recommendation = recommendation.expect("an enabled entry gets a recommendation");

            assert_eq!(recommendation.risk, RiskBucket::NeedsReview, "{verdict:?}");
            assert!(
                !recommendation.reason.contains(STARTUP_NOT_RATED_SAFE),
                "{verdict:?} never was a Safe candidate: {}",
                recommendation.reason
            );
        }
    }

    #[test]
    fn windows_own_and_disabled_startup_entries_still_get_no_recommendation() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let brain = app_brain(scratch.path());
        let program = harmless_program(scratch.path());

        let os_owned = startup::StartupEntry {
            os_owned: true,
            os_reason: Some("This is part of Windows.".to_owned()),
            ..startup_entry(Some(program.clone()), startup::Verdict::SafeToDisable)
        };
        let disabled = startup::StartupEntry {
            enabled: false,
            ..startup_entry(Some(program), startup::Verdict::SafeToDisable)
        };

        for entry in [os_owned, disabled] {
            let (finding, recommendation) = startup_recommendation(&entry, &brain);
            assert!(recommendation.is_none(), "{}: no action offered", entry.id);
            assert_eq!(finding.module, "startup");
        }
    }
}
