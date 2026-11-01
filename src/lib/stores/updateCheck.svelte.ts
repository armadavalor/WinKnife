/**
 * Opt-in "new version available" reminder (`core/update_check.rs`).
 *
 * Off until the user says yes. The backend makes the one GitHub request and
 * decides what is due; this store only mirrors its state. Every failure is
 * swallowed: a reminder must never get in the way of the app.
 */
import {
  updateCheckOpen,
  updateCheckRun,
  updateCheckSetEnabled,
  updateCheckSkip,
  updateCheckSnooze,
  updateCheckState,
} from "$lib/api";
import type { UpdateState } from "$lib/types";

class UpdateCheckStore {
  /** Null until the backend has answered. */
  state = $state<UpdateState | null>(null);

  /** Load the saved state, then run the (throttled) check in the background. */
  async init(): Promise<void> {
    await this.apply(updateCheckState());
    if (this.state?.enabled) await this.apply(updateCheckRun());
  }

  /** The first-run question is shown once, until it has been answered. */
  get askFirst(): boolean {
    return this.state !== null && !this.state.prompted;
  }

  async setEnabled(enabled: boolean): Promise<void> {
    await this.apply(updateCheckSetEnabled(enabled));
    if (enabled) await this.apply(updateCheckRun());
  }

  snooze(): Promise<void> {
    return this.apply(updateCheckSnooze());
  }

  skip(): Promise<void> {
    return this.apply(updateCheckSkip());
  }

  async download(): Promise<void> {
    try {
      await updateCheckOpen();
    } catch {
      // Nothing to show: the banner stays so the user can try again.
    }
  }

  private async apply(next: Promise<UpdateState>): Promise<void> {
    try {
      this.state = await next;
    } catch {
      // Silent by design.
    }
  }
}

export const updateCheck = new UpdateCheckStore();
