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
  quarantineRestore,
  quarantineStats,
} from "$lib/api";
import {
  isApiError,
  type QuarantineBatch,
  type QuarantineItem,
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
  stats = $state<QuarantineStats>({ itemCount: 0, totalBytes: 0 });
  /** Where the fetch is. */
  status = $state<LoadStatus>("idle");
  /** Message of the last failure. */
  error = $state<string | null>(null);
  /** Whether the drawer panel is showing. */
  open = $state(false);
  /** Items of the expanded batch, keyed by batch id. */
  items = $state<Record<string, QuarantineItem[]>>({});
  /** Result of the last restore, shown inline. */
  lastRestore = $state<RestoreReport | null>(null);

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
      this.error = message(error);
    }
  }

  /** Put a whole batch back. */
  async restore(batchId: string): Promise<void> {
    this.error = null;
    try {
      this.lastRestore = await quarantineRestore(batchId);
      await this.reload();
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  /** Run the retention purge and the size cap now. */
  async purge(): Promise<void> {
    this.error = null;
    try {
      await quarantinePurge();
      await this.reload();
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  /**
   * Keep the badge live. The backend emits `quarantine:changed` after every
   * clean, restore and purge, so the drawer never shows a stale total.
   */
  async listen(): Promise<() => void> {
    const unlisten = await onQuarantineChanged((payload) => {
      this.stats = {
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
