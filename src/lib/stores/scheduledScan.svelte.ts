/**
 * Scheduled scan (C11): saved schedule, last report, and the in-app check.
 *
 * There is no OS scheduled task (K-021): while the app is open the shell asks
 * the backend every few minutes whether a slot is due, and once on start so a
 * slot missed while closed still runs. The backend decides "due" and refuses
 * overlapping runs; this store only asks and shows the result.
 */
import { scheduledScanGet, scheduledScanRunDue, scheduledScanSet } from "$lib/api";
import {
  isApiError,
  type ScheduledReport,
  type ScheduledScanSettings,
  type ScheduledScanState,
} from "$lib/types";

/** How often the open app asks whether a slot is due. */
export const SCHEDULED_CHECK_MS = 5 * 60 * 1000;

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class ScheduledScanStore {
  state = $state<ScheduledScanState | null>(null);
  /** A report from a run this session that the banner has not had dismissed. */
  banner = $state<ScheduledReport | null>(null);
  saving = $state(false);
  error = $state<string | null>(null);

  private checking = false;

  async refresh(): Promise<void> {
    try {
      this.state = await scheduledScanGet();
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  async save(settings: ScheduledScanSettings): Promise<boolean> {
    this.saving = true;
    this.error = null;
    try {
      this.state = await scheduledScanSet(settings);
      return true;
    } catch (error: unknown) {
      this.error = message(error);
      return false;
    } finally {
      this.saving = false;
    }
  }

  /** Ask the backend to run a due slot. Never overlaps itself. */
  async checkDue(): Promise<void> {
    if (this.checking) return;
    this.checking = true;
    try {
      const report = await scheduledScanRunDue();
      if (report) {
        this.banner = report;
        await this.refresh();
      }
    } catch (error: unknown) {
      // Background check: log it, the page shows the state on its own.
      console.warn("scheduled scan check failed", message(error));
    } finally {
      this.checking = false;
    }
  }

  /** Start the on-start check and the periodic one; returns the stop. */
  start(): () => void {
    void this.checkDue();
    const timer = setInterval(() => void this.checkDue(), SCHEDULED_CHECK_MS);
    return () => clearInterval(timer);
  }

  dismissBanner(): void {
    this.banner = null;
  }
}

export const scheduledScanStore = new ScheduledScanStore();
