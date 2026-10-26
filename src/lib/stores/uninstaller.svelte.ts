/**
 * The uninstaller-leftover finder's scan → review → execute state.
 *
 * Nothing is selected by default: a leftover folder is at most Review, and
 * only quarantine recommendations can be selected at all.
 */
import {
  cancelScan,
  execute as invokeExecute,
  onScanProgress,
  startUninstallerScan,
} from "$lib/api";
import {
  isApiError,
  type LeftoverScanResult,
  type ExecReport,
  type Finding,
  type LeftoverMeta,
  type Recommendation,
} from "$lib/types";
import { ExecTracker } from "./execProgress.svelte";

/** Which step of the 3-step pattern the route is on. */
export type Step = "idle" | "scanning" | "review" | "executing" | "done";

const MODULE_ID = "uninstaller";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class UninstallerStore {
  step = $state<Step>("idle");
  result = $state<LeftoverScanResult | null>(null);
  /** Selected recommendation ids. */
  selected = $state<Set<string>>(new Set());
  currentPath = $state<string | null>(null);
  pct = $state<number | null>(null);
  stopping = $state(false);
  report = $state<ExecReport | null>(null);
  /** Progress and Stop for the running execute. */
  readonly exec = new ExecTracker();
  error = $state<string | null>(null);

  private runningScanId: string | null = null;
  private unlistenProgress: (() => void) | null = null;

  /** The recommendation covering one finding, when there is one. */
  recommendationFor(finding: Finding): Recommendation | undefined {
    return this.result?.recommendations.find((rec) =>
      rec.findingIds.includes(finding.id),
    );
  }

  /** Whether a recommendation may be selected for removal. */
  isSelectable(rec: Recommendation | undefined): rec is Recommendation {
    return rec !== undefined && rec.action === "quarantine";
  }

  /** File count and flags the scan recorded for one folder. */
  metaOf(finding: Finding): LeftoverMeta {
    const meta = finding.meta as Partial<LeftoverMeta> | null;
    return {
      files: meta?.files ?? 0,
      hasReparse: meta?.hasReparse ?? false,
      incomplete: meta?.incomplete ?? false,
    };
  }

  /** Run a scan. */
  async scan(): Promise<void> {
    this.step = "scanning";
    this.error = null;
    this.report = null;
    this.pct = null;
    this.currentPath = null;
    this.stopping = false;
    this.runningScanId = null;
    this.selected = new Set();
    try {
      await this.attachProgress();
      this.result = await startUninstallerScan();
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

  /**
   * Ask the running scan to stop. The backend names the scan in an initial
   * progress event as soon as it starts; a Stop pressed before that event is
   * sent the moment it arrives (see `attachProgress`).
   */
  async stop(): Promise<void> {
    this.stopping = true;
    if (this.runningScanId) {
      await cancelScan(this.runningScanId);
    }
  }

  /** Toggle one folder. */
  toggle(recommendationId: string, next: boolean): void {
    const selected = new Set(this.selected);
    if (next) {
      selected.add(recommendationId);
    } else {
      selected.delete(recommendationId);
    }
    this.selected = selected;
  }

  /** Select every folder that may be removed. */
  selectAll(): void {
    this.selected = new Set(
      (this.result?.recommendations ?? [])
        .filter((rec) => this.isSelectable(rec))
        .map((rec) => rec.id),
    );
  }

  /** Clear the selection. */
  selectNone(): void {
    this.selected = new Set();
  }

  /** Apply the selection. */
  async execute(): Promise<void> {
    const scan = this.result;
    if (!scan) {
      return;
    }
    this.step = "executing";
    this.error = null;
    try {
      this.report = await this.exec.run(scan.scanId, () =>
        invokeExecute(scan.scanId, [...this.selected], false),
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
      this.pct = payload.pct;
    });
  }

  private detachProgress(): void {
    this.unlistenProgress?.();
    this.unlistenProgress = null;
  }
}

/** Shared uninstaller-leftover finder state. */
export const uninstallerStore = new UninstallerStore();
