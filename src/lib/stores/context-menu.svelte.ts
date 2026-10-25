/**
 * Context-menu cleaner state (C2).
 *
 * Disabling goes through `execute`, so it passes the executor's gates and lands
 * in the audit log. Re-enabling is the reversal and has its own command, as
 * tweak undo does: it puts back what was recorded before the write.
 */
import { contextMenuScan, contextMenuUndo, execute } from "$lib/api";
import { t } from "$lib/i18n";
import {
  isApiError,
  type ContextMenuEntry,
  type ContextMenuScanResult,
  type Recommendation,
} from "$lib/types";

import { itemNote } from "./execNotes";

/** Lifecycle of the read. */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

/** Prefix the backend puts on a context-menu entry's tweak id. */
const TWEAK_PREFIX = "context-menu:";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class ContextMenuStore {
  /** Every menu entry found, both hives. */
  entries = $state<ContextMenuEntry[]>([]);
  /** Where the read is. */
  status = $state<LoadStatus>("idle");
  /** Message of the last failure. */
  error = $state<string | null>(null);
  /** Id of the entry currently being changed, so its button can say so. */
  busy = $state<string | null>(null);
  /** What the last change did, in words. */
  lastReport = $state<string | null>(null);

  /** Scan id the disable action needs. */
  private scanId: string | null = null;
  /** Recommendation per entry id, built by Rust at scan time. */
  private byEntry = new Map<string, Recommendation>();

  #loaded = false;
  #inflight: Promise<void> | null = null;

  /** The recommendation that disables `entry`, or `null` when there is none. */
  recommendationOf(entry: ContextMenuEntry): Recommendation | null {
    return this.byEntry.get(entry.id) ?? null;
  }

  /** Read, unless it has already been done. */
  load(): Promise<void> {
    if (this.#loaded) {
      return Promise.resolve();
    }
    return this.reload();
  }

  /** Share refreshes; a mutation performs its own fresh read. */
  reload(): Promise<void> {
    if (this.busy !== null) return Promise.resolve();
    this.#inflight ??= this.#doReload().finally(() => {
      this.#inflight = null;
    });
    return this.#inflight;
  }

  async #doReload(): Promise<void> {
    this.status = "loading";
    this.error = null;
    try {
      this.apply(await contextMenuScan());
      this.status = "ready";
      this.#loaded = true;
    } catch (error: unknown) {
      this.entries = [];
      this.error = message(error);
      this.status = "error";
      this.#loaded = false;
    }
  }

  private apply(result: ContextMenuScanResult): void {
    this.scanId = result.scanId;
    this.entries = result.entries;
    this.byEntry = new Map();
    for (const recommendation of result.recommendations) {
      const justification = recommendation.justification;
      if (justification.kind === "tweak" && justification.tweakId.startsWith(TWEAK_PREFIX)) {
        this.byEntry.set(justification.tweakId.slice(TWEAK_PREFIX.length), recommendation);
      }
    }
  }

  /** Hide an entry, through the executor. */
  async disable(entry: ContextMenuEntry): Promise<void> {
    if (this.busy !== null || this.status === "loading") return;
    const recommendation = this.byEntry.get(entry.id);
    if (this.scanId === null || !recommendation) {
      this.error = t("exec.entryStale");
      return;
    }
    this.busy = entry.id;
    this.error = null;
    try {
      const report = await execute(this.scanId, [recommendation.id], false);
      const item = report.items[0];
      this.lastReport = item ? itemNote(item, t("exec.needsAdmin")) : null;
      await this.#doReload();
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = null;
    }
  }

  /** Put back what was there before `disable`. */
  async enable(entry: ContextMenuEntry, done: string): Promise<void> {
    if (this.busy !== null || this.status === "loading") return;
    this.busy = entry.id;
    this.error = null;
    try {
      await contextMenuUndo(entry.id);
      this.lastReport = done;
      await this.#doReload();
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = null;
    }
  }
}

/** Shared context-menu state. */
export const contextMenuStore = new ContextMenuStore();
