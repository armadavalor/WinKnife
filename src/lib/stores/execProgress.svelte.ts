/**
 * Progress and Stop for one execute batch (issue #39).
 *
 * The cleaner, duplicates and empty-folders routes all move a selection to
 * quarantine through the same `execute` command, so they share this tracker
 * rather than three copies of the same listener.
 */
import { cancelScan, onExecProgress } from "$lib/api";

export class ExecTracker {
  /** Items handled so far. */
  done = $state(0);
  /** Items in the batch; null until the first progress event. */
  total = $state<number | null>(null);
  /** Path being moved right now. */
  currentPath = $state<string | null>(null);
  /** Stop was pressed and the batch has not finished yet. */
  stopping = $state(false);

  private scanId: string | null = null;
  /** Stop was pressed before the backend had registered the batch. */
  private cancelPending = false;

  /**
   * Run `work` (the execute call for `scanId`) while listening for its
   * progress. The listener goes away however `work` ends.
   */
  async run<T>(scanId: string, work: () => Promise<T>): Promise<T> {
    this.scanId = scanId;
    this.done = 0;
    this.total = null;
    this.currentPath = null;
    this.stopping = false;
    this.cancelPending = false;
    const unlisten = await onExecProgress((payload) => {
      if (payload.scanId !== scanId) {
        return;
      }
      if (this.cancelPending) {
        this.cancelPending = false;
        void cancelScan(scanId);
      }
      this.done = payload.itemsDone;
      this.total = payload.itemsTotal;
      this.currentPath = payload.currentPath;
    });
    try {
      return await work();
    } finally {
      unlisten();
      this.scanId = null;
      this.stopping = false;
    }
  }

  /**
   * Ask the running batch to stop before its next item. What already moved
   * stays in quarantine; the rest is left where it was.
   */
  async stop(): Promise<void> {
    if (!this.scanId || this.stopping) {
      return;
    }
    this.stopping = true;
    // The batch registers its stop token only once it starts; a press before
    // that is resent with the first progress event.
    this.cancelPending = !(await cancelScan(this.scanId));
  }

  /** Completion percentage, or null while the total is unknown. */
  get pct(): number | null {
    return this.total ? Math.floor((this.done / this.total) * 100) : null;
  }
}
