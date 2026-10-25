/**
 * Whether WinKnife runs as administrator, and restarting it as one (K-020).
 *
 * Never elevates by itself: the restart happens only on a click, through the
 * UAC prompt. A "No" there leaves this window running with a notice.
 */
import { elevationStatus, onNeedsAdmin, restartAsAdmin } from "$lib/api";
import { isApiError } from "$lib/types";

export type ElevationNotice = "declined" | "failed" | null;

/** The page to reopen after the restart; null for the dashboard. */
export function moduleOfPath(pathname: string): string | null {
  const first = pathname.split("/").filter(Boolean)[0] ?? "";
  return first === "" || first === "dashboard" ? null : first;
}

class ElevationStore {
  /** Null until the backend has answered. */
  elevated = $state<boolean | null>(null);
  /** An action failed for lack of rights since the last dismiss. */
  needsAdmin = $state(false);
  busy = $state(false);
  notice = $state<ElevationNotice>(null);
  /** Detail of the last failed restart. */
  error = $state<string | null>(null);

  private listening = false;

  async init(): Promise<void> {
    if (!this.listening) {
      this.listening = true;
      onNeedsAdmin(() => {
        if (this.elevated !== true) this.needsAdmin = true;
      });
    }
    try {
      this.elevated = await elevationStatus();
    } catch {
      // Unknown is shown as "standard": the restart offer stays harmless.
      this.elevated = false;
    }
  }

  dismiss(): void {
    this.needsAdmin = false;
    this.notice = null;
  }

  /** Ask Windows to restart WinKnife elevated on `pathname`'s page. */
  async restart(pathname: string): Promise<void> {
    this.busy = true;
    this.notice = null;
    this.error = null;
    try {
      const outcome = await restartAsAdmin(moduleOfPath(pathname));
      if (outcome.status === "declined") this.notice = "declined";
      if (outcome.status === "failed") {
        this.notice = "failed";
        this.error = `Windows error ${outcome.code}`;
      }
    } catch (error: unknown) {
      this.notice = "failed";
      this.error = isApiError(error) ? error.message : String(error);
    } finally {
      this.busy = false;
    }
  }
}

export const elevation = new ElevationStore();
