/**
 * The only module in the UI that talks to Rust.
 *
 * Components import typed wrappers from here and never call `invoke` or spell
 * an event name themselves (CONVENTIONS.md). Keeping the boundary in one file
 * means a renamed command or event breaks the build once, here, instead of
 * silently at runtime in five routes.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AppSettings,
  CachedScan,
  CleanerSummary,
  DashboardSummary,
  DriveInfo,
  DuplicateScanResult,
  ExecProgress,
  ExecReport,
  ModuleManifest,
  MoveWarning,
  PowerAction,
  PurgeReport,
  QuarantineBatch,
  QuarantineChanged,
  QuarantineItem,
  QuarantineStats,
  RestoreOutcome,
  RestoreReport,
  ScanDone,
  ScanProgress,
  ScanResult,
  ScheduleMode,
  ShutdownTick,
  StartupScanResult,
  StartupSource,
  TimerStatus,
  TreeLevel,
  TreemapScanResult,
} from "$lib/types";

/** Event names, mirroring the constants in `core/events.rs` and `lib.rs`. */
export const EVENTS = {
  scanProgress: "scan:progress",
  scanDone: "scan:done",
  execProgress: "exec:progress",
  quarantineChanged: "quarantine:changed",
  shutdownTick: "shutdown:tick",
  launchNavigate: "launch:navigate",
} as const;

/* -------------------------------------------------------------------------
 * Commands — registry
 * ---------------------------------------------------------------------- */

/**
 * Manifests of every registered module, in sidebar order.
 *
 * Rejects with an {@link import('$lib/types').ApiError}.
 */
export function listModules(): Promise<ModuleManifest[]> {
  return invoke<ModuleManifest[]>("list_modules");
}

/** Everything the dashboard shows, in one round trip. */
export function dashboardSummary(): Promise<DashboardSummary> {
  return invoke<DashboardSummary>("dashboard_summary");
}

/* -------------------------------------------------------------------------
 * Commands — cleaner (scan → review → execute)
 * ---------------------------------------------------------------------- */

/** Run a read-only scan and return its findings and recommendations. */
export function startScan(moduleId: string): Promise<ScanResult> {
  return invoke<ScanResult>("start_scan", { moduleId });
}

/**
 * Ask a running scan to stop.
 *
 * Resolves `false` when the scan already finished — the user pressed stop as the
 * last file went through, which is not an error.
 */
export function cancelScan(scanId: string): Promise<boolean> {
  return invoke<boolean>("cancel_scan", { scanId });
}

/* -------------------------------------------------------------------------
 * Commands — duplicates
 * ---------------------------------------------------------------------- */

/**
 * Run a read-only duplicate scan.
 *
 * `roots` overrides the saved selection for this run only; omit it to use the
 * saved roots, or the profile folders when nothing is saved.
 */
export function startDuplicateScan(
  roots?: string[],
): Promise<DuplicateScanResult> {
  return invoke<DuplicateScanResult>("start_duplicate_scan", { roots });
}

/* -------------------------------------------------------------------------
 * Commands — treemap
 * ---------------------------------------------------------------------- */

/** Drives the treemap can map, with their capacity. */
export function treemapDrives(): Promise<DriveInfo[]> {
  return invoke<DriveInfo[]>("treemap_drives");
}

/**
 * Walk a drive, aggregate it, cache it and return its top level.
 *
 * Long-running and cancellable with {@link cancelScan} using the returned
 * `scanId`; progress arrives on `scan:progress`.
 */
export function treemapScan(root: string): Promise<TreemapScanResult> {
  return invoke<TreemapScanResult>("treemap_scan", { root });
}

/**
 * One directory's children, sized from the cached scan of `root`.
 *
 * This is the zoom step: the directory itself is re-read from disk, so file
 * names are current, and the folder totals come from the cache.
 */
export function treemapChildren(
  root: string,
  path: string,
): Promise<TreeLevel> {
  return invoke<TreeLevel>("treemap_children", { root, path });
}

/** The cached header for a drive, or null when it has never been scanned. */
export function treemapCached(root: string): Promise<CachedScan | null> {
  return invoke<CachedScan | null>("treemap_cached", { root });
}

/** Show a path in File Explorer. Opens a window; changes nothing. */
export function revealInExplorer(path: string): Promise<void> {
  return invoke<void>("reveal_in_explorer", { path });
}

/* -------------------------------------------------------------------------
 * Commands — startup manager
 * ---------------------------------------------------------------------- */

/** Enumerate everything that runs at logon. Read-only. */
export function startupScan(): Promise<StartupScanResult> {
  return invoke<StartupScanResult>("startup_scan");
}

/**
 * Turn an autostart entry back on.
 *
 * The reversal. Turning something **off** goes through {@link execute}, so it
 * passes the executor's gates and is written to the audit log; putting it back
 * has nothing left to re-verify, exactly like restoring from quarantine.
 * Resolves to what the user should know about a shortcut that came back only
 * in part; empty otherwise.
 */
export function startupEnable(
  source: StartupSource,
  key: string,
): Promise<MoveWarning[]> {
  return invoke<MoveWarning[]>("startup_enable", { source, key });
}

/* -------------------------------------------------------------------------
 * Commands — settings
 * ---------------------------------------------------------------------- */

/** Everything the settings page shows. */
export function getSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_settings");
}

/**
 * Save the settings page.
 *
 * Resolves with the settings **as stored**, which is not always what was sent:
 * Rust re-validates the retention window and the duplicate size floor, so the
 * page must render what it gets back rather than what it asked for.
 */
export function setSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke<AppSettings>("set_settings", { settingsIn: settings });
}

/** Re-fetch a scan the app already ran. */
export function getRecommendations(scanId: string): Promise<ScanResult> {
  return invoke<ScanResult>("get_recommendations", { scanId });
}

/** Green/yellow/red totals for the header line. */
export function scanSummary(scanId: string): Promise<CleanerSummary> {
  return invoke<CleanerSummary>("scan_summary", { scanId });
}

/**
 * Apply the recommendations the user approved, by recommendation id.
 *
 * `confirmEmptyRecycleBin` answers a separate question. Everything else in a
 * batch goes to quarantine and can be undone; emptying the bin is final and
 * takes every drive with it, so Rust refuses to do it on the strength of a
 * selected id alone. Only set it when the confirm dialog named the recycle bin
 * and the user went ahead.
 */
export function execute(
  scanId: string,
  approved: string[],
  confirmEmptyRecycleBin: boolean,
): Promise<ExecReport> {
  return invoke<ExecReport>("execute", {
    scanId,
    approved,
    confirmEmptyRecycleBin,
  });
}

/* -------------------------------------------------------------------------
 * Commands — quarantine drawer
 * ---------------------------------------------------------------------- */

/** Batches that still hold something, newest first. */
export function quarantineList(): Promise<QuarantineBatch[]> {
  return invoke<QuarantineBatch[]>("quarantine_list");
}

/** Items of one batch. */
export function quarantineBatchItems(
  batchId: string,
): Promise<QuarantineItem[]> {
  return invoke<QuarantineItem[]>("quarantine_batch_items", { batchId });
}

/** Totals across everything still held. */
export function quarantineStats(): Promise<QuarantineStats> {
  return invoke<QuarantineStats>("quarantine_stats");
}

/** Restore a whole batch. */
export function quarantineRestore(batchId: string): Promise<RestoreReport> {
  return invoke<RestoreReport>("quarantine_restore", { batchId });
}

/** Restore one item. */
export function quarantineRestoreItem(
  itemId: string,
): Promise<RestoreOutcome> {
  return invoke<RestoreOutcome>("quarantine_restore_item", { itemId });
}

/** Run the retention purge and the size cap now. */
export function quarantinePurge(): Promise<PurgeReport> {
  return invoke<PurgeReport>("quarantine_purge");
}

/**
 * Permanently remove one item whose encrypted restore was refused. The
 * backend refuses any other item (decision b, 2026-10-12).
 */
export function quarantinePurgeItem(itemId: string): Promise<PurgeReport> {
  return invoke<PurgeReport>("quarantine_purge_item", { itemId });
}

/* -------------------------------------------------------------------------
 * Commands — shutdown timer
 * ---------------------------------------------------------------------- */

/** Arm a timer, replacing any existing one. */
export function shutdownSchedule(
  mode: ScheduleMode,
  action: PowerAction,
): Promise<TimerStatus> {
  return invoke<TimerStatus>("shutdown_schedule", { mode, action });
}

/** Cancel the timer. Cancelling when nothing is armed is not an error. */
export function shutdownCancel(): Promise<TimerStatus> {
  return invoke<TimerStatus>("shutdown_cancel");
}

/** Current timer state. */
export function shutdownStatus(): Promise<TimerStatus> {
  return invoke<TimerStatus>("shutdown_status");
}

/* -------------------------------------------------------------------------
 * Commands — launch (`--module <id>`)
 * ---------------------------------------------------------------------- */

/**
 * The route a launch asked for and the UI has not opened yet, or `null`.
 * Taking it clears it. Already validated: an unknown id arrives as the
 * dashboard.
 */
export function takeLaunchRoute(): Promise<string | null> {
  return invoke<string | null>("take_launch_route");
}

/* -------------------------------------------------------------------------
 * Events
 * ---------------------------------------------------------------------- */

function subscribe<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return listen<T>(event, (message) => handler(message.payload));
}

/** Subscribe to scan progress. Resolves with the unsubscribe function. */
export function onScanProgress(
  handler: (payload: ScanProgress) => void,
): Promise<UnlistenFn> {
  return subscribe(EVENTS.scanProgress, handler);
}

/** Subscribe to scan completion. */
export function onScanDone(
  handler: (payload: ScanDone) => void,
): Promise<UnlistenFn> {
  return subscribe(EVENTS.scanDone, handler);
}

/** Subscribe to execute progress. */
export function onExecProgress(
  handler: (payload: ExecProgress) => void,
): Promise<UnlistenFn> {
  return subscribe(EVENTS.execProgress, handler);
}

/** Subscribe to quarantine changes. */
export function onQuarantineChanged(
  handler: (payload: QuarantineChanged) => void,
): Promise<UnlistenFn> {
  return subscribe(EVENTS.quarantineChanged, handler);
}

/** Subscribe to the once-a-second shutdown countdown. */
export function onShutdownTick(
  handler: (payload: ShutdownTick) => void,
): Promise<UnlistenFn> {
  return subscribe(EVENTS.shutdownTick, handler);
}

/**
 * A second launch forwarded a `--module`: call {@link takeLaunchRoute}.
 * No payload; the route is pulled so it is opened once.
 */
export function onLaunchNavigate(handler: () => void): Promise<UnlistenFn> {
  return subscribe<null>(EVENTS.launchNavigate, () => handler());
}
