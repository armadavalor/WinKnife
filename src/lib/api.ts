/**
 * The only module in the UI that talks to Rust.
 *
 * Components import typed wrappers from here and never call `invoke` or spell
 * an event name themselves (CONVENTIONS.md). Keeping the boundary in one file
 * means a renamed command or event breaks the build once, here, instead of
 * silently at runtime in five routes.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import type {
  AppSettings,
  CachedScan,
  CleanerSummary,
  CreateOutcome,
  CloseReport,
  DashboardSummary,
  DriveInfo,
  DuplicateScanResult,
  EnvChange,
  EnvListing,
  EnvPreview,
  ProfilePreview,
  EmptyFolderScanResult,
  ExecProgress,
  ExecReport,
  FolderGroupUpdate,
  KeepRule,
  HostsBackup,
  HostsDocument,
  PortRow,
  HostsPreview,
  HostsSaveOutcome,
  ModuleManifest,
  MoveWarning,
  PanelInfo,
  PowerAction,
  PurgeReport,
  QuarantineBatch,
  QuarantineChanged,
  QuarantineItem,
  QuarantineStats,
  RestoreOutcome,
  RestoreProgress,
  RestorePointStatus,
  RestoreReport,
  ScanDone,
  ScanProgress,
  ScanResult,
  ScheduleMode,
  KeepAwakeStatus,
  ShutdownTick,
  StartupScanResult,
  StartupSource,
  TimerStatus,
  TreeLevel,
  TreemapScanResult,
  TweaksScanResult,
  ContextMenuScanResult,
  UpdateStatus,
  UnlockerReport,
  RelaunchOutcome,
  AppsList,
  SignerInfo,
} from "$lib/types";
import { isApiError } from "$lib/types";

/* -------------------------------------------------------------------------
 * Administrator rights (K-021)
 * ---------------------------------------------------------------------- */

const needsAdminHandlers = new Set<() => void>();

/** Called whenever a command fails for lack of administrator rights. */
export function onNeedsAdmin(handler: () => void): () => void {
  needsAdminHandlers.add(handler);
  return () => needsAdminHandlers.delete(handler);
}

function reportsNeedsAdmin(result: unknown): boolean {
  const items = (result as { items?: unknown } | null)?.items;
  return (
    Array.isArray(items) &&
    items.some(
      (item: ExecReport["items"][number]) =>
        item?.outcome?.status === "failed" && item.outcome.needsAdmin === true,
    )
  );
}

/** Every command goes through here so a needs-admin failure reaches the banner. */
async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const result = await tauriInvoke<T>(command, args);
    if (reportsNeedsAdmin(result)) needsAdminHandlers.forEach((handler) => handler());
    return result;
  } catch (error: unknown) {
    if (isApiError(error) && error.code === "needs_admin") {
      needsAdminHandlers.forEach((handler) => handler());
    }
    throw error;
  }
}

/** Whether WinKnife runs with administrator rights. */
export function elevationStatus(): Promise<boolean> {
  return invoke<boolean>("elevation_status");
}

/**
 * Restart through the UAC prompt, reopening `module` (null = dashboard). On
 * `started` this window closes; on `declined` nothing changed.
 */
export function restartAsAdmin(module: string | null): Promise<RelaunchOutcome> {
  return invoke<RelaunchOutcome>("restart_as_admin", { module });
}

/** Event names, mirroring the constants in `core/events.rs` and `lib.rs`. */
export const EVENTS = {
  scanProgress: "scan:progress",
  scanDone: "scan:done",
  execProgress: "exec:progress",
  quarantineChanged: "quarantine:changed",
  restoreProgress: "quarantine:restore-progress",
  shutdownTick: "shutdown:tick",
  launchNavigate: "launch:navigate",
  keepAwakeChanged: "keep-awake:changed",
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

/** Change which folder of one duplicate-folder group stays. */
export function setFolderKeepRule(
  scanId: string,
  groupId: string,
  rule: KeepRule,
): Promise<FolderGroupUpdate> {
  return invoke<FolderGroupUpdate>("set_folder_keep_rule", {
    scanId,
    groupId,
    rule,
  });
}

/** Hide a duplicate-folder group from future scans. */
export function ignoreFolderGroup(id: string): Promise<void> {
  return invoke<void>("ignore_folder_group", { id });
}

/** Bring every ignored duplicate-folder group back. */
export function clearIgnoredFolderGroups(): Promise<void> {
  return invoke<void>("clear_ignored_folder_groups");
}

/* -------------------------------------------------------------------------
 * Commands — empty folders
 * ---------------------------------------------------------------------- */

/**
 * Run a read-only scan for empty folders under `root` (the user profile when
 * omitted). Cancellable with {@link cancelScan}; progress arrives on
 * `scan:progress`.
 */
export function startEmptyFolderScan(
  root?: string,
): Promise<EmptyFolderScanResult> {
  return invoke<EmptyFolderScanResult>("start_empty_folder_scan", { root });
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
 * Commands — tweaks
 * ---------------------------------------------------------------------- */

/** Read every catalogue tweak's live value. Read-only. */
export function tweaksScan(): Promise<TweaksScanResult> {
  return invoke<TweaksScanResult>("tweaks_scan");
}

/**
 * Put a tweak's recorded prior value back.
 *
 * The reversal. Applying goes through {@link execute}; undo writes back exactly
 * what was there before (or removes the value if it was not set), and refuses
 * when the value was changed since.
 */
export function tweaksUndo(tweakId: string): Promise<void> {
  return invoke<void>("tweaks_undo", { tweakId });
}

/** What is holding Windows Update back, if anything. Read-only. */
export function tweaksUpdateStatus(): Promise<UpdateStatus> {
  return invoke<UpdateStatus>("tweaks_update_status");
}

/** Save the applied tweaks and the settings as a profile file. */
export function profileExport(path: string): Promise<void> {
  return invoke<void>("profile_export", { path });
}

/**
 * Check a profile file and preview it. Writes nothing: apply its tweaks with
 * {@link execute} and its settings with {@link setSettings}.
 */
export function profilePreview(path: string): Promise<ProfilePreview> {
  return invoke<ProfilePreview>("profile_preview", { path });
}

/* -------------------------------------------------------------------------
 * Commands — context menu
 * ---------------------------------------------------------------------- */

/** Read every right-click menu entry and who owns it. Read-only. */
export function contextMenuScan(): Promise<ContextMenuScanResult> {
  return invoke<ContextMenuScanResult>("context_menu_scan");
}

/**
 * Re-enable an entry WinKnife disabled, by putting back what was recorded
 * before the write. Disabling goes through {@link execute}.
 */
export function contextMenuUndo(entryId: string): Promise<void> {
  return invoke<void>("context_menu_undo", { entryId });
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

/**
 * Restore a whole batch. Progress comes on `quarantine:restore-progress`;
 * `cancelScan(batchId)` stops it between items and the rest stays held.
 */
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
 * Permanently remove held items: one, a selection or a whole batch (K-020).
 * Only call this after the drawer's "cannot be undone" dialog; the backend
 * refuses without the confirmation flag.
 */
export function quarantinePurgeItems(itemIds: string[]): Promise<PurgeReport> {
  return invoke<PurgeReport>("quarantine_purge_items", { itemIds, confirmed: true });
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
 * Commands — restore point (module spec 09)
 * ---------------------------------------------------------------------- */

/** System Protection status. Read-only. */
export function restorePointStatus(): Promise<RestorePointStatus> {
  return invoke<RestorePointStatus>("restore_point_status");
}

/**
 * Create a restore point through the elevated helper. Windows shows a UAC
 * prompt; the promise settles when the helper exits (or after 5 minutes).
 */
export function restorePointCreate(description: string): Promise<CreateOutcome> {
  return invoke<CreateOutcome>("restore_point_create", { description });
}

/** Open Windows' own System Protection dialog. Only on a button click. */
export function restorePointOpenSettings(): Promise<void> {
  return invoke<void>("restore_point_open_settings");
}

/* -------------------------------------------------------------------------
 * Commands — hosts file (K-019)
 * ---------------------------------------------------------------------- */

/** The hosts file, parsed. Read-only. */
export function hostsRead(): Promise<HostsDocument> {
  return invoke<HostsDocument>("hosts_read");
}

/** Validate `text` and diff it against the file on disk. Writes nothing. */
export function hostsPreview(text: string): Promise<HostsPreview> {
  return invoke<HostsPreview>("hosts_preview", { text });
}

/**
 * Back up, then write `text` through the elevated helper. Windows shows a UAC
 * prompt; the promise settles when the helper exits.
 */
export function hostsSave(text: string): Promise<HostsSaveOutcome> {
  return invoke<HostsSaveOutcome>("hosts_save", { text });
}

/** Backups WinKnife took, newest first. */
export function hostsBackups(): Promise<HostsBackup[]> {
  return invoke<HostsBackup[]>("hosts_backups");
}

/** Write a backup back over the hosts file (backs up the current one first). */
export function hostsRestore(name: string): Promise<HostsSaveOutcome> {
  return invoke<HostsSaveOutcome>("hosts_restore", { name });
}

/** Show a backup in Explorer. */
export function hostsRevealBackup(name: string): Promise<void> {
  return invoke<void>("hosts_reveal_backup", { name });
}

/** Run `ipconfig /flushdns`. Only on a button click; returns its output. */
export function hostsFlushDns(): Promise<string> {
  return invoke<string>("hosts_flush_dns");
}

/** Listening ports and open connections with their owning process. Read-only. */
export function networkList(): Promise<PortRow[]> {
  return invoke<PortRow[]>("network_list");
}

/* -------------------------------------------------------------------------
 * Commands — environment variables (C15, K-024)
 * ---------------------------------------------------------------------- */

/** User and system variables, PATH-like ones split and flagged. Read-only. */
export function envVarsList(): Promise<EnvListing> {
  return invoke<EnvListing>("env_vars_list");
}

/** Validate a change and diff it. Writes nothing. */
export function envVarsPreview(change: EnvChange): Promise<EnvPreview> {
  return invoke<EnvPreview>("env_vars_preview", { change });
}

/** Write a user variable (or delete it), recording the prior value for undo. */
export function envVarsApply(change: EnvChange): Promise<void> {
  return invoke<void>("env_vars_apply", { change });
}

/** Put back what the last change to `name` replaced. */
export function envVarsUndo(name: string): Promise<void> {
  return invoke<void>("env_vars_undo", { name });
}

/* -------------------------------------------------------------------------
 * Commands — legacy panels (module spec 10)
 * ---------------------------------------------------------------------- */

/** The fixed catalogue with each entry's availability. Read-only. */
export function legacyPanelsList(): Promise<PanelInfo[]> {
  return invoke<PanelInfo[]>("legacy_panels_list");
}

/** Open one of Windows' own windows by id. Only on a button click. */
export function legacyPanelsOpen(id: string): Promise<void> {
  return invoke<void>("legacy_panels_open", { id });
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
 * Commands — keep awake
 * ---------------------------------------------------------------------- */

/** Keep the machine awake for `minutes` (1 to 1440), replacing any session. */
export function keepAwakeStart(
  minutes: number,
  display: boolean,
): Promise<KeepAwakeStatus> {
  return invoke<KeepAwakeStatus>("keep_awake_start", { minutes, display });
}

/** Stop keeping the machine awake. Stopping when idle is not an error. */
export function keepAwakeStop(): Promise<KeepAwakeStatus> {
  return invoke<KeepAwakeStatus>("keep_awake_stop");
}

/** Current keep-awake state. */
export function keepAwakeStatus(): Promise<KeepAwakeStatus> {
  return invoke<KeepAwakeStatus>("keep_awake_status");
}

/* -------------------------------------------------------------------------
 * modules/file_unlocker.rs
 * ---------------------------------------------------------------------- */

/**
 * Which programs hold `paths` open. Read-only: nothing is closed or changed.
 * Folders are walked (links not followed) up to the module's file cap.
 */
export function unlockerQuery(paths: string[]): Promise<UnlockerReport> {
  return invoke<UnlockerReport>("unlocker_query", { paths });
}

/**
 * Politely ask one program to close (the same request Windows sends at
 * logoff). Never forces: the program may ask to save, or decline. Rust
 * refuses services, Explorer, system-critical processes and a PID whose
 * start time no longer matches.
 */
export function unlockerRequestClose(
  pid: number,
  startTime: string,
): Promise<CloseReport> {
  return invoke<CloseReport>("unlocker_request_close", { pid, startTime });
}

/** Native picker for one or more files. Resolves to `[]` when cancelled. */
export async function pickFiles(): Promise<string[]> {
  const picked = await openDialog({ multiple: true, directory: false });
  return normalisePicked(picked);
}

/** Native picker for one folder. Resolves to `[]` when cancelled. */
export async function pickFolder(): Promise<string[]> {
  const picked = await openDialog({ multiple: false, directory: true });
  return normalisePicked(picked);
}

const PROFILE_FILTERS = [{ name: "WinKnife profile", extensions: ["json"] }];

/** Native picker for where to save a profile. Resolves to `null` when cancelled. */
export function pickProfileSave(): Promise<string | null> {
  return saveDialog({ defaultPath: "winknife-profile.json", filters: PROFILE_FILTERS });
}

/** Native picker for a profile to import. Resolves to `null` when cancelled. */
export async function pickProfileOpen(): Promise<string | null> {
  const picked = await openDialog({ multiple: false, directory: false, filters: PROFILE_FILTERS });
  return normalisePicked(picked)[0] ?? null;
}

function normalisePicked(picked: string | string[] | null): string[] {
  if (picked === null) {
    return [];
  }
  return Array.isArray(picked) ? picked : [picked];
}

/** Drag-and-drop over the window: hover state and dropped paths. */
export interface DropHandlers {
  onHover: (hovering: boolean) => void;
  onDrop: (paths: string[]) => void;
}

/** Subscribe to files dropped on the window. Resolves with the unsubscribe function. */
export function onFileDrop(handlers: DropHandlers): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    if (payload.type === "enter" || payload.type === "over") {
      handlers.onHover(true);
    } else if (payload.type === "leave") {
      handlers.onHover(false);
    } else if (payload.type === "drop") {
      handlers.onHover(false);
      handlers.onDrop(payload.paths);
    }
  });
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

/** Subscribe to batch-restore progress. */
export function onRestoreProgress(
  handler: (payload: RestoreProgress) => void,
): Promise<UnlistenFn> {
  return subscribe(EVENTS.restoreProgress, handler);
}

/** Subscribe to quarantine changes. */
export function onQuarantineChanged(
  handler: (payload: QuarantineChanged) => void,
): Promise<UnlistenFn> {
  return subscribe(EVENTS.quarantineChanged, handler);
}

/** Subscribe to keep-awake start, stop and expiry. */
export function onKeepAwakeChanged(handler: () => void): Promise<UnlistenFn> {
  return subscribe(EVENTS.keepAwakeChanged, handler);
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

/* Commands — installed apps (read-only) */

export function appsList(): Promise<AppsList> {
  return invoke<AppsList>("apps_list");
}

/** Authenticode signer of one Win32 entry, looked up on demand. */
export function appsSigner(id: string): Promise<SignerInfo> {
  return invoke<SignerInfo>("apps_signer", { id });
}

/** Opens Settings > Apps & features. WinKnife never uninstalls. */
export function appsOpenSettings(): Promise<void> {
  return invoke<void>("apps_open_settings");
}
