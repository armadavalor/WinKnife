/**
 * Quarantine drawer state.
 *
 * Global rather than per-route: the drawer is part of the shell (ARCHITECTURE
 * section 7), and a clean started from the cleaner route has to update the
 * badge whichever route the user is looking at when it finishes.
 */
import {
  onQuarantineChanged,
  quarantineBatchItems,
  quarantineList,
  quarantinePurge,
  quarantinePurgeItem,
  quarantineRestore,
  quarantineStats,
} from "$lib/api";
import { removalFailure, type RemovalFailure } from "$lib/stores/execNotes";
import {
  isApiError,
  type QuarantineBatch,
  type QuarantineItem,
  type PurgeReport,
  type QuarantineStats,
  type RestoreReport,
} from "$lib/types";

/** Lifecycle of the drawer's fetch. */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class QuarantineStore {
  /** Batches still holding something, newest first. */
  batches = $state<QuarantineBatch[]>([]);
  /** Totals, used for the sidebar badge. */
  stats = $state<QuarantineStats>({
    itemCount: 0,
    totalBytes: 0,
    unreadableRows: 0,
    refusedItems: 0,
    refusedBytes: 0,
    capBytes: 0,
  });
  /** Where the fetch is. */
  status = $state<LoadStatus>("idle");
  /** Message of the last failed fetch. */
  error = $state<string | null>(null);
  /**
   * Message of the last refused or failed action (restore, purge, batch
   * items). Kept apart from `error` because the reload that follows another
   * operation's `quarantine:changed` would otherwise wipe it before the user
   * could read it: a restore refused as Busy vanished this way (e2e QA).
   */
  actionError = $state<string | null>(null);
  /** Whether the drawer panel is showing. */
  open = $state(false);
  /** Items of the expanded batch, keyed by batch id. */
  items = $state<Record<string, QuarantineItem[]>>({});
  /** Result of the last restore, shown inline. */
  lastRestore = $state<RestoreReport | null>(null);
  /**
   * Result of the last purge, shown inline.
   *
   * The size cap evicts oldest-first across batch boundaries, so a batch can
   * lose part of itself and still appear in the list at its old size. Saying so
   * is the point (architect addition B, phase 3): a partially evicted batch is
   * exactly the thing an honest tool admits to rather than quietly rounding off.
   */
  lastPurge = $state<PurgeReport | null>(null);
  /**
   * A refused item the user tried to remove by hand and could not. It has its
   * own note: the purge's "retried on the next purge" is false for an item
   * the automatic purge never touches (round 3).
   */
  removeFailed = $state<{ path: string; failure: RemovalFailure } | null>(null);

  /** Fetch batches and totals. */
  async reload(): Promise<void> {
    this.status = "loading";
    this.error = null;
    try {
      const [batches, stats] = await Promise.all([
        quarantineList(),
        quarantineStats(),
      ]);
      this.batches = batches;
      this.stats = stats;
      this.status = "ready";
    } catch (error: unknown) {
      this.batches = [];
      this.error = message(error);
      this.status = "error";
    }
  }

  /** Show the drawer and refresh it. */
  async show(): Promise<void> {
    this.open = true;
    await this.reload();
  }

  /** Hide the drawer. */
  hide(): void {
    this.open = false;
    this.lastRestore = null;
    this.lastPurge = null;
    this.removeFailed = null;
    this.actionError = null;
  }

  /** Load (or drop) the item list of one batch. */
  async toggleBatch(batchId: string): Promise<void> {
    if (this.items[batchId]) {
      const { [batchId]: _dropped, ...rest } = this.items;
      this.items = rest;
      return;
    }
    try {
      this.items = { ...this.items, [batchId]: await quarantineBatchItems(batchId) };
    } catch (error: unknown) {
      this.actionError = message(error);
    }
  }

  /** Put a whole batch back. */
  async restore(batchId: string): Promise<void> {
    this.actionError = null;
    try {
      this.lastRestore = await quarantineRestore(batchId);
      await this.reload();
    } catch (error: unknown) {
      this.actionError = message(error);
    }
  }

  /** Run the retention purge and the size cap now. */
  async purge(): Promise<void> {
    this.actionError = null;
    try {
      this.lastPurge = await quarantinePurge();
      await this.reload();
    } catch (error: unknown) {
      this.actionError = message(error);
    }
  }

  /**
   * Permanently remove one item whose encrypted restore was refused, after the
   * user confirmed it. The automatic purge never removes such an item
   * (decision b, 2026-10-12).
   */
  async purgeItem(item: QuarantineItem): Promise<void> {
    this.actionError = null;
    this.removeFailed = null;
    let failure: RemovalFailure | null;
    try {
      const report = await quarantinePurgeItem(item.id);
      failure = removalFailure(item.id, report);
    } catch (error: unknown) {
      failure = removalFailure(item.id, { error });
    }
    if (failure) {
      this.removeFailed = { path: item.quarantinePath, failure };
    }
    try {
      const remaining = await quarantineBatchItems(item.batchId);
      this.items = { ...this.items, [item.batchId]: remaining };
      await this.reload();
    } catch (error: unknown) {
      this.actionError = message(error);
    }
  }

  /**
   * Keep the badge live. The backend emits `quarantine:changed` after every
   * clean, restore and purge, so the drawer never shows a stale total.
   */
  async listen(): Promise<() => void> {
    const unlisten = await onQuarantineChanged((payload) => {
      // The event carries the badge numbers only; the corrupt-row count comes
      // from `reload`, which asks the ledger directly.
      // The refused bytes and the cap also come from `reload`.
      this.stats = {
        ...this.stats,
        itemCount: payload.itemCount,
        totalBytes: payload.totalBytes,
      };
      if (this.open) {
        void this.reload();
      }
    });
    return unlisten;
  }
}

/** Shared drawer state. */
export const quarantineStore = new QuarantineStore();
