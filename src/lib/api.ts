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
  ExecProgress,
  ModuleManifest,
  QuarantineChanged,
  ScanDone,
  ScanProgress,
} from "$lib/types";

/** Event names, mirroring the constants in `core/events.rs`. */
export const EVENTS = {
  scanProgress: "scan:progress",
  scanDone: "scan:done",
  execProgress: "exec:progress",
  quarantineChanged: "quarantine:changed",
} as const;

/* -------------------------------------------------------------------------
 * Commands
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
