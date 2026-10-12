/**
 * The cleaner's scan → review → execute state.
 *
 * The three steps are one state machine rather than three screens, because the
 * user is always somewhere in the same sentence: "here is what I found, here is
 * what you picked, here is what happened".
 */
import {
  execute as invokeExecute,
  onScanProgress,
  scanSummary,
  startScan,
} from "$lib/api";
import {
  isApiError,
  type CleanerSummary,
  type ExecReport,
  type Finding,
  type Recommendation,
  type ScanResult,
} from "$lib/types";
import { initialSelection } from "./selection";

/** Which step of the 3-step pattern the route is on. */
export type Step = "idle" | "scanning" | "review" | "executing" | "done";

/** A recommendation paired with the finding it acts on. */
export interface Row {
  recommendation: Recommendation;
  finding: Finding;
}

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class ScanStore {
  /** Current step. */
  step = $state<Step>("idle");
  /** The scan, once it has run. */
  result = $state<ScanResult | null>(null);
  /** Bucket totals. */
  summary = $state<CleanerSummary | null>(null);
  /** Selected recommendation ids. */
  selected = $state<Set<string>>(new Set());
  /** Path the scanner is currently looking at. */
  currentPath = $state<string | null>(null);
  /** Items walked so far. */
  itemsScanned = $state(0);
  /** Report of the last execute. */
  report = $state<ExecReport | null>(null);
  /** Message of the last failure. */
  error = $state<string | null>(null);

  private unlistenProgress: (() => void) | null = null;

  /** Every recommendation paired with its finding, in scan order. */
  get rows(): Row[] {
    const scan = this.result;
    if (!scan) {
      return [];
    }
    const byId = new Map(scan.findings.map((finding) => [finding.id, finding]));
    return scan.recommendations.flatMap((recommendation) => {
      const finding = byId.get(recommendation.findingIds[0]);
      return finding ? [{ recommendation, finding }] : [];
    });
  }

  /** Rows in one bucket. */
  rowsIn(risk: Recommendation["risk"]): Row[] {
    return this.rows.filter((row) => row.recommendation.risk === risk);
  }

  /** Bytes the current selection would reclaim. */
  get selectedBytes(): number {
    return this.rows
      .filter((row) => this.selected.has(row.recommendation.id))
      .reduce((total, row) => total + row.recommendation.savingsBytes, 0);
  }

  /** Whether the selection includes the recycle bin, which is not reversible. */
  get selectionEmptiesRecycleBin(): boolean {
    return this.rows.some(
      (row) =>
        this.selected.has(row.recommendation.id) &&
        row.finding.kind === "recycleBinItem",
    );
  }

  /** Run a scan. Green items start selected, except the recycle bin. */
  async scan(moduleId: string): Promise<void> {
    this.step = "scanning";
    this.error = null;
    this.report = null;
    this.itemsScanned = 0;
    this.currentPath = null;
    await this.attachProgress();

    try {
      const result = await startScan(moduleId);
      this.result = result;
      this.selected = initialSelection(result.recommendations, result.findings);
      this.summary = await scanSummary(result.scanId);
      this.step = "review";
    } catch (error: unknown) {
      this.error = message(error);
      this.step = "idle";
    } finally {
      this.detachProgress();
    }
  }

  /** Toggle one row. */
  toggle(recommendationId: string, next: boolean): void {
    const selected = new Set(this.selected);
    if (next) {
      selected.add(recommendationId);
    } else {
      selected.delete(recommendationId);
    }
    this.selected = selected;
  }

  /** Toggle a whole group. */
  toggleGroup(recommendationIds: string[], next: boolean): void {
    const selected = new Set(this.selected);
    for (const id of recommendationIds) {
      if (next) {
        selected.add(id);
      } else {
        selected.delete(id);
      }
    }
    this.selected = selected;
  }

  /**
   * Apply the selection.
   *
   * `confirmEmptyRecycleBin` is passed through from the dialog rather than
   * derived here: it must mean "the user was shown the recycle-bin warning and
   * went ahead", not "a recycle-bin row happens to be ticked".
   */
  async execute(confirmEmptyRecycleBin: boolean): Promise<void> {
    const scan = this.result;
    if (!scan) {
      return;
    }
    this.step = "executing";
    this.error = null;
    try {
      this.report = await invokeExecute(
        scan.scanId,
        [...this.selected],
        confirmEmptyRecycleBin,
      );
      this.step = "done";
    } catch (error: unknown) {
      this.error = message(error);
      this.step = "review";
    }
  }

  /** Back to the start, ready for another scan. */
  reset(): void {
    this.step = "idle";
    this.result = null;
    this.summary = null;
    this.selected = new Set();
    this.report = null;
    this.error = null;
  }

  private async attachProgress(): Promise<void> {
    this.detachProgress();
    this.unlistenProgress = await onScanProgress((payload) => {
      this.itemsScanned = payload.itemsScanned;
      this.currentPath = payload.currentPath;
    });
  }

  private detachProgress(): void {
    this.unlistenProgress?.();
    this.unlistenProgress = null;
  }
}

/** Shared cleaner state. */
export const scanStore = new ScanStore();
