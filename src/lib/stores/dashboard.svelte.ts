/**
 * Dashboard state: one round trip for everything the landing screen shows.
 *
 * One command rather than five, because the dashboard is a single glance and
 * five independent loading states arriving in five different orders is a worse
 * experience than one that appears at once.
 */
import { dashboardSummary } from "$lib/api";
import {
  isApiError,
  type DashboardSummary,
  type ScanRecord,
} from "$lib/types";

/** Lifecycle of the fetch. */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

class DashboardStore {
  /** Everything the page shows, or null before the first fetch. */
  summary = $state<DashboardSummary | null>(null);
  /** Where the fetch is. */
  status = $state<LoadStatus>("idle");
  /** Message of the last failure. */
  error = $state<string | null>(null);

  /** Fetch unless it is already in flight. */
  async load(): Promise<void> {
    if (this.status === "loading") {
      return;
    }
    this.status = "loading";
    this.error = null;
    try {
      this.summary = await dashboardSummary();
      this.status = "ready";
    } catch (error: unknown) {
      this.error = isApiError(error) ? error.message : String(error);
      this.status = "error";
    }
  }

  /** The last run of one module, if it has ever run. */
  lastScan(moduleId: string): ScanRecord | null {
    return (
      this.summary?.modules.find((card) => card.module === moduleId)
        ?.lastScan ?? null
    );
  }

  /** Total bytes the modules currently believe they could reclaim. */
  get reclaimable(): number {
    return (this.summary?.modules ?? []).reduce(
      (total, card) => total + (card.lastScan?.reclaimableBytes ?? 0),
      0,
    );
  }
}

/** Shared dashboard state. */
export const dashboardStore = new DashboardStore();
