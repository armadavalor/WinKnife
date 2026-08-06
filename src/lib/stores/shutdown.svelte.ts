/**
 * Shutdown timer state.
 *
 * The countdown shown here is driven by the `shutdown:tick` event rather than a
 * browser interval, so the UI and the Rust timer can never disagree about how
 * long is left — the backend is the only clock that matters.
 */
import {
  onShutdownTick,
  shutdownCancel,
  shutdownSchedule,
  shutdownStatus,
} from "$lib/api";
import {
  isApiError,
  type PowerAction,
  type ScheduleMode,
  type TimerStatus,
} from "$lib/types";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class ShutdownStore {
  /** Current timer state as the backend reported it. */
  status = $state<TimerStatus>({ active: null, dryRun: false });
  /** Seconds left, refreshed once a second by `shutdown:tick`. */
  remainingSecs = $state(0);
  /** Message of the last failure. */
  error = $state<string | null>(null);
  /** Whether a schedule/cancel call is in flight. */
  busy = $state(false);

  private unlisten: (() => void) | null = null;

  /** Whether a timer is armed. */
  get isArmed(): boolean {
    return this.status.active !== null;
  }

  /** Fetch the current state. */
  async refresh(): Promise<void> {
    try {
      this.status = await shutdownStatus();
      this.remainingSecs = this.status.active?.remainingSecs ?? 0;
      this.error = null;
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  /** Arm a timer, replacing any existing one. */
  async schedule(mode: ScheduleMode, action: PowerAction): Promise<void> {
    this.busy = true;
    this.error = null;
    try {
      this.status = await shutdownSchedule(mode, action);
      this.remainingSecs = this.status.active?.remainingSecs ?? 0;
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = false;
    }
  }

  /** Cancel the timer. */
  async cancel(): Promise<void> {
    this.busy = true;
    this.error = null;
    try {
      this.status = await shutdownCancel();
      this.remainingSecs = 0;
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = false;
    }
  }

  /** Follow the backend countdown. Returns the unsubscribe function. */
  async listen(): Promise<() => void> {
    this.unlisten?.();
    this.unlisten = await onShutdownTick((payload) => {
      this.remainingSecs = payload.remainingSecs;
      // The backend disarms itself at zero; pick that up without polling.
      if (payload.remainingSecs === 0 && this.isArmed) {
        void this.refresh();
      }
    });
    return () => {
      this.unlisten?.();
      this.unlisten = null;
    };
  }
}

/** Shared timer state. */
export const shutdownStore = new ShutdownStore();
