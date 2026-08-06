/**
 * The duplicate finder's scan → review → execute state.
 *
 * Two things make it different from the cleaner's store:
 *
 * - **Nothing is selected by default.** A duplicate is a judgement about a pair,
 *   and the app has no way to know which copy somebody meant to keep. The
 *   heuristic nominates; the selection stays empty until a person agrees.
 * - **A scan can be stopped.** A full-hash pass over a Pictures folder is
 *   minutes of reading, so `cancel_scan` is reachable while it runs.
 */
import {
  cancelScan,
  execute as invokeExecute,
  onScanProgress,
  startDuplicateScan,
} from "$lib/api";
import {
  isApiError,
  type DuplicateCopy,
  type DuplicateScanResult,
  type DuplicateSet,
  type ExecReport,
  type ExtensionClass,
  type Recommendation,
} from "$lib/types";

/** Which step of the 3-step pattern the route is on. */
export type Step = "idle" | "scanning" | "review" | "executing" | "done";

const IMAGE_EXTENSIONS = new Set([
  "jpg",
  "jpeg",
  "png",
  "gif",
  "bmp",
  "webp",
  "heic",
  "tif",
  "tiff",
  "raw",
  "cr2",
  "nef",
  "svg",
]);
const VIDEO_EXTENSIONS = new Set([
  "mp4",
  "mkv",
  "avi",
  "mov",
  "wmv",
  "flv",
  "webm",
  "m4v",
  "mpg",
  "mpeg",
]);
const DOCUMENT_EXTENSIONS = new Set([
  "pdf",
  "doc",
  "docx",
  "xls",
  "xlsx",
  "ppt",
  "pptx",
  "txt",
  "rtf",
  "odt",
  "ods",
  "csv",
  "md",
  "epub",
]);

function extensionOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot <= 0 ? "" : name.slice(dot + 1).toLowerCase();
}

/** Which filter bucket a path falls into. */
export function classOf(path: string): Exclude<ExtensionClass, "all"> {
  const extension = extensionOf(path);
  if (IMAGE_EXTENSIONS.has(extension)) return "images";
  if (VIDEO_EXTENSIONS.has(extension)) return "video";
  if (DOCUMENT_EXTENSIONS.has(extension)) return "documents";
  return "other";
}

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class DuplicatesStore {
  step = $state<Step>("idle");
  result = $state<DuplicateScanResult | null>(null);
  /** Selected recommendation ids. */
  selected = $state<Set<string>>(new Set());
  /** Per-set keeper override: set id → finding id of the copy to keep. */
  keeperOverrides = $state<Map<string, string>>(new Map());
  currentPath = $state<string | null>(null);
  itemsScanned = $state(0);
  pct = $state<number | null>(null);
  stopping = $state(false);
  report = $state<ExecReport | null>(null);
  error = $state<string | null>(null);

  /** Filter: extension class. */
  filterClass = $state<ExtensionClass>("all");
  /** Filter: smallest set size to show, in bytes. */
  filterMinBytes = $state(0);

  private unlistenProgress: (() => void) | null = null;

  /** Sets after the filters, still sorted by wasted bytes. */
  get sets(): DuplicateSet[] {
    const sets = this.result?.scan.sets ?? [];
    return sets.filter((set) => {
      if (set.sizeBytes < this.filterMinBytes) {
        return false;
      }
      if (this.filterClass === "all") {
        return true;
      }
      return set.copies.some(
        (copy) => classOf(copy.path) === this.filterClass,
      );
    });
  }

  /** Whether a copy is the keeper, honouring a per-set override. */
  isKeeper(set: DuplicateSet, copy: DuplicateCopy): boolean {
    const override = this.keeperOverrides.get(set.id);
    return override === undefined ? copy.keeper : override === copy.findingId;
  }

  /** The recommendation covering one copy, when there is one. */
  recommendationFor(copy: DuplicateCopy): Recommendation | undefined {
    return this.result?.recommendations.find((rec) =>
      rec.findingIds.includes(copy.findingId),
    );
  }

  /** Bytes the current selection would reclaim. */
  get selectedBytes(): number {
    return (this.result?.recommendations ?? [])
      .filter((rec) => this.selected.has(rec.id))
      .reduce((total, rec) => total + rec.savingsBytes, 0);
  }

  /** Run a scan. */
  async scan(): Promise<void> {
    this.step = "scanning";
    this.error = null;
    this.report = null;
    this.itemsScanned = 0;
    this.pct = null;
    this.currentPath = null;
    this.stopping = false;
    this.selected = new Set();
    this.keeperOverrides = new Map();
    await this.attachProgress();

    try {
      this.result = await startDuplicateScan();
      this.step = "review";
    } catch (error: unknown) {
      this.error = message(error);
      this.step = "idle";
    } finally {
      this.detachProgress();
      this.stopping = false;
    }
  }

  /** Ask the running scan to stop. */
  async stop(): Promise<void> {
    const scanId = this.result?.scanId;
    this.stopping = true;
    if (scanId) {
      await cancelScan(scanId);
      return;
    }
    // The scan is still running, so its id has not come back yet. Rust hands the
    // id over with the result; until then the only honest thing is to say we are
    // stopping and let the in-flight scan finish its current file.
  }

  /** Toggle one copy. Keepers are never selectable. */
  toggle(recommendationId: string, next: boolean): void {
    const selected = new Set(this.selected);
    if (next) {
      selected.add(recommendationId);
    } else {
      selected.delete(recommendationId);
    }
    this.selected = selected;
  }

  /** Choose a different keeper for one set, and unselect it if it was picked. */
  chooseKeeper(set: DuplicateSet, copy: DuplicateCopy): void {
    const overrides = new Map(this.keeperOverrides);
    overrides.set(set.id, copy.findingId);
    this.keeperOverrides = overrides;

    const rec = this.recommendationFor(copy);
    if (rec) {
      this.toggle(rec.id, false);
    }
  }

  /** Select every copy that is not its set's keeper. */
  selectAllNonKeepers(): void {
    const selected = new Set<string>();
    for (const set of this.sets) {
      for (const copy of set.copies) {
        if (this.isKeeper(set, copy)) {
          continue;
        }
        const rec = this.recommendationFor(copy);
        if (rec && rec.action === "quarantine") {
          selected.add(rec.id);
        }
      }
    }
    this.selected = selected;
  }

  /** Clear the selection. */
  selectNone(): void {
    this.selected = new Set();
  }

  /** Bulk switch: keep the newest (or oldest) copy in every visible set. */
  keepBy(which: "newest" | "oldest"): void {
    const overrides = new Map(this.keeperOverrides);
    for (const set of this.sets) {
      const sorted = [...set.copies].sort((a, b) => {
        const left = Date.parse(a.modified ?? a.created ?? "") || 0;
        const right = Date.parse(b.modified ?? b.created ?? "") || 0;
        return which === "newest" ? right - left : left - right;
      });
      const chosen = sorted[0];
      if (chosen) {
        overrides.set(set.id, chosen.findingId);
      }
    }
    this.keeperOverrides = overrides;
    // The selection is rebuilt because a copy that was selected may now be the
    // keeper, and offering to remove the copy the user just chose to keep is
    // exactly the mistake this module must not make.
    this.selectAllNonKeepers();
  }

  /** Apply the selection. Duplicates never empty the recycle bin. */
  async execute(): Promise<void> {
    const scan = this.result;
    if (!scan) {
      return;
    }
    this.step = "executing";
    this.error = null;
    try {
      this.report = await invokeExecute(scan.scanId, [...this.selected], false);
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
    this.keeperOverrides = new Map();
    this.report = null;
    this.error = null;
  }

  private async attachProgress(): Promise<void> {
    this.detachProgress();
    this.unlistenProgress = await onScanProgress((payload) => {
      if (payload.module !== "duplicates") {
        return;
      }
      this.itemsScanned = payload.itemsScanned;
      this.currentPath = payload.currentPath;
      this.pct = payload.pct;
    });
  }

  private detachProgress(): void {
    this.unlistenProgress?.();
    this.unlistenProgress = null;
  }
}

/** Shared duplicate-finder state. */
export const duplicatesStore = new DuplicatesStore();
