/**
 * Keep-awake state.
 *
 * The backend owns the session and its wall-clock deadline; this store only
 * mirrors it. The countdown is computed from that deadline on a local tick, so
 * a laptop that slept (and woke) shows the true time left, not a drifted one.
 */
import {
  keepAwakeStart,
  keepAwakeStatus,
  keepAwakeStop,
  onKeepAwakeChanged,
} from "$lib/api";
import { isApiError, type KeepAwakeStatus } from "$lib/types";

const IDLE: KeepAwakeStatus = { active: false, deadline: null, display: false };
const TICK_MS = 1000;

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class KeepAwakeStore {
  /** Session state as the backend reported it. */
  status = $state<KeepAwakeStatus>(IDLE);
  /** Seconds left until the deadline. */
  remainingSecs = $state(0);
  /** Message of the last failure. */
  error = $state<string | null>(null);
  /** Whether a start/stop call is in flight. */
  busy = $state(false);

  private timer: ReturnType<typeof setInterval> | null = null;

  private apply(status: KeepAwakeStatus): void {
    this.status = status;
    this.updateRemaining();
  }

  private updateRemaining(): void {
    const deadline = this.status.deadline;
    this.remainingSecs = deadline
      ? Math.max(0, Math.ceil((Date.parse(deadline) - Date.now()) / 1000))
      : 0;
  }

  /** Fetch the current state. */
  async refresh(): Promise<void> {
    try {
      this.apply(await keepAwakeStatus());
      this.error = null;
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  /** Start (or restart) a session. */
  async start(minutes: number, display: boolean): Promise<void> {
    this.busy = true;
    this.error = null;
    try {
      this.apply(await keepAwakeStart(minutes, display));
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = false;
    }
  }

  /** Stop the session. */
  async stop(): Promise<void> {
    this.busy = true;
    this.error = null;
    try {
      this.apply(await keepAwakeStop());
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = false;
    }
  }

  /** Follow backend changes and tick the countdown. Returns the unsubscribe. */
  async listen(): Promise<() => void> {
    this.stopTimer();
    this.timer = setInterval(() => this.updateRemaining(), TICK_MS);
    const unlisten = await onKeepAwakeChanged(() => {
      void this.refresh();
    });
    return () => {
      unlisten();
      this.stopTimer();
    };
  }

  private stopTimer(): void {
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }
}

/** Shared keep-awake state. */
export const keepAwakeStore = new KeepAwakeStore();
