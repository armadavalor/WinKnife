/**
 * The big-file archiver's scan → review → move state.
 *
 * Nothing is selected by default: moving a file is a judgement. Files in a
 * protected location are listed for information only and cannot be selected.
 */
import { archiveFiles, cancelScan, onScanProgress, startArchiverScan } from "$lib/api";
import {
  isApiError,
  type ArchiveReport,
  type ArchiverMeta,
  type ArchiverScanResult,
  type Finding,
  type Recommendation,
} from "$lib/types";

/** Which step of the 3-step pattern the route is on. */
export type Step = "idle" | "scanning" | "review" | "moving" | "done";

const MODULE_ID = "archiver";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class ArchiverStore {
  step = $state<Step>("idle");
  result = $state<ArchiverScanResult | null>(null);
  /** Selected finding ids. */
  selected = $state<Set<string>>(new Set());
  /** Folder to scan; empty means the user profile. */
  root = $state("");
  /** Folder to move the chosen files to. */
  destination = $state("");
  currentPath = $state<string | null>(null);
  stopping = $state(false);
  report = $state<ArchiveReport | null>(null);
  error = $state<string | null>(null);

  private runningScanId: string | null = null;
  private unlistenProgress: (() => void) | null = null;

  /** The recommendation covering one finding, when there is one. */
  recommendationFor(finding: Finding): Recommendation | undefined {
    return this.result?.recommendations.find((rec) =>
      rec.findingIds.includes(finding.id),
    );
  }

  /** Whether a finding may be selected for moving. */
  isSelectable(rec: Recommendation | undefined): rec is Recommendation {
    return rec !== undefined && rec.risk !== "doNotDelete";
  }

  /** Whole days since the file was last used. */
  ageOf(finding: Finding): number {
    return (finding.meta as ArchiverMeta | null)?.ageDays ?? 0;
  }

  /** Bytes of the selected files. */
  get selectedBytes(): number {
    return (this.result?.findings ?? [])
      .filter((finding) => this.selected.has(finding.id))
      .reduce((sum, finding) => sum + finding.sizeBytes, 0);
  }

  /** Run a scan. */
  async scan(): Promise<void> {
    this.step = "scanning";
    this.error = null;
    this.report = null;
    this.currentPath = null;
    this.stopping = false;
    this.runningScanId = null;
    this.selected = new Set();
    try {
      await this.attachProgress();
      const root = this.root.trim();
      this.result = await startArchiverScan(root === "" ? undefined : root);
      this.step = "review";
    } catch (error: unknown) {
      this.error = message(error);
      this.step = "idle";
    } finally {
      this.detachProgress();
      this.stopping = false;
      this.runningScanId = null;
    }
  }

  /** Ask the running scan to stop (see `attachProgress` for an early Stop). */
  async stop(): Promise<void> {
    this.stopping = true;
    if (this.runningScanId) {
      await cancelScan(this.runningScanId);
    }
  }

  /** Toggle one file. */
  toggle(findingId: string, next: boolean): void {
    const selected = new Set(this.selected);
    if (next) {
      selected.add(findingId);
    } else {
      selected.delete(findingId);
    }
    this.selected = selected;
  }

  /** Move the selection to the destination. */
  async move(): Promise<void> {
    const scan = this.result;
    if (!scan) {
      return;
    }
    this.step = "moving";
    this.error = null;
    try {
      this.report = await archiveFiles(
        scan.scanId,
        [...this.selected],
        this.destination.trim(),
      );
      this.step = "done";
    } catch (error: unknown) {
      this.error = message(error);
      this.step = "review";
    }
  }

  /** Back to the start. */
  reset(): void {
    this.step = "idle";
    this.result = null;
    this.selected = new Set();
    this.report = null;
    this.error = null;
  }

  private async attachProgress(): Promise<void> {
    this.detachProgress();
    this.unlistenProgress = await onScanProgress((payload) => {
      if (payload.module !== MODULE_ID) {
        return;
      }
      // A Stop pressed before the first event had no id to send; send it now.
      const isFirstEvent = this.runningScanId === null;
      this.runningScanId = payload.scanId;
      if (isFirstEvent && this.stopping) {
        void cancelScan(payload.scanId);
      }
      this.currentPath = payload.currentPath;
    });
  }

  private detachProgress(): void {
    this.unlistenProgress?.();
    this.unlistenProgress = null;
  }
}

/** Shared big-file archiver state. */
export const archiverStore = new ArchiverStore();
