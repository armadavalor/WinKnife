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
  CleanerSummary,
  ExecProgress,
  ExecReport,
  ModuleManifest,
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
  TimerStatus,
} from "$lib/types";

/** Event names, mirroring the constants in `core/events.rs` and `lib.rs`. */
export const EVENTS = {
  scanProgress: "scan:progress",
  scanDone: "scan:done",
  execProgress: "exec:progress",
  quarantineChanged: "quarantine:changed",
  shutdownTick: "shutdown:tick",
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

/* -------------------------------------------------------------------------
 * Commands — cleaner (scan → review → execute)
 * ---------------------------------------------------------------------- */

/** Run a read-only scan and return its findings and recommendations. */
export function startScan(moduleId: string): Promise<ScanResult> {
  return invoke<ScanResult>("start_scan", { moduleId });
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
