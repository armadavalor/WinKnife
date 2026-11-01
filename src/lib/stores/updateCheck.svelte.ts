/**
 * Opt-in "new version available" reminder (`core/update_check.rs`).
 *
 * Off until the user says yes. The backend makes the one GitHub request and
 * decides what is due; this store only mirrors its state. A failed check is
 * swallowed: a reminder must never get in the way of the app. Installing only
 * happens after "Update now" and a confirm, and its failure is shown.
 */
import {
  onUpdateProgress,
  updateInstall,
  updateCheckOpen,
  updateCheckRun,
  updateCheckSetEnabled,
  updateCheckSkip,
  updateCheckSnooze,
  updateCheckState,
} from "$lib/api";
import type { UpdateProgress, UpdateState } from "$lib/types";

class UpdateCheckStore {
  /** Null until the backend has answered. */
  state = $state<UpdateState | null>(null);
  /** Banner flow: idle → confirming → installing → (failed → release page). */
  phase = $state<"idle" | "confirming" | "installing" | "failed">("idle");
  progress = $state<UpdateProgress>({ downloaded: 0, total: null });
  error = $state<string | null>(null);

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

  /** Open the release page in the browser (also the fallback when installing fails). */
  async download(): Promise<void> {
    try {
      await updateCheckOpen();
    } catch {
      // Nothing to show: the banner stays so the user can try again.
    }
  }

  /** "Update now" was clicked: ask for confirmation before anything is fetched. */
  confirm(): void {
    this.phase = "confirming";
    this.error = null;
  }

  cancel(): void {
    this.phase = "idle";
  }

  /** Download, verify and install. On success the installer restarts the app. */
  async install(): Promise<void> {
    this.phase = "installing";
    this.error = null;
    this.progress = { downloaded: 0, total: null };
    const unlisten = await onUpdateProgress((p) => (this.progress = p)).catch(
      () => null,
    );
    try {
      await updateInstall();
    } catch (err) {
      this.error = (err as { message?: string })?.message ?? String(err);
      this.phase = "failed";
    } finally {
      unlisten?.();
    }
  }

  /** Download fraction 0..1, or null while the total size is unknown. */
  get fraction(): number | null {
    const { downloaded, total } = this.progress;
    return total ? Math.min(1, downloaded / total) : null;
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
