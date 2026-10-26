/**
 * The duplicate finder's scan → review → execute state.
 *
 * Two things make it different from the cleaner's store:
 *
 * - **Nothing is selected by default.** A duplicate is a judgement about a pair,
 *   and the app has no way to know which copy somebody meant to keep. The
 *   heuristic nominates; the selection stays empty until a person agrees.
 *   Folder groups are the exception: a whole folder that repeats another one
 *   is a clearer call, so its non-kept copies start selected (C3). Similar
 *   images (C18) are not: look-alikes are never byte-equal, so they start
 *   with nothing selected.
 * - **A scan can be stopped.** A full-hash pass over a Pictures folder is
 *   minutes of reading, so `cancel_scan` is reachable while it runs.
 */
import {
  cancelScan,
  execute as invokeExecute,
  ignoreFolderGroup,
  onScanProgress,
  setFolderKeepRule,
  setSimilarImageKeeper,
  startDuplicateScan,
  startSimilarImageScan,
} from "$lib/api";
import {
  isApiError,
  type DuplicateCopy,
  type DuplicateScanResult,
  type DuplicateSet,
  type ExecReport,
  type ExtensionClass,
  type FolderGroup,
  type ImageGroup,
  type ImageMember,
  type KeepRule,
  type Recommendation,
  type SimilarImageScanResult,
} from "$lib/types";
import { ExecTracker } from "./execProgress.svelte";

/** Which step of the 3-step pattern the route is on. */
export type Step = "idle" | "scanning" | "review" | "executing" | "done";

/** Byte-equal copies, or look-alike images (C18). */
export type DuplicateMode = "files" | "images";

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

function folderRecIds(group: FolderGroup): string[] {
  return group.members.flatMap((member) => member.recommendationIds);
}

class DuplicatesStore {
  step = $state<Step>("idle");
  result = $state<DuplicateScanResult | null>(null);
  /** Which kind of scan the idle step starts and the review step shows. */
  mode = $state<DuplicateMode>("files");
  /** The last similar-image scan, when `mode` is "images". */
  imageResult = $state<SimilarImageScanResult | null>(null);
  /** Selected recommendation ids. */
  selected = $state<Set<string>>(new Set());
  /** Per-set keeper override: set id → finding id of the copy to keep. */
  keeperOverrides = $state<Map<string, string>>(new Map());
  currentPath = $state<string | null>(null);
  itemsScanned = $state(0);
  pct = $state<number | null>(null);
  stopping = $state(false);
  report = $state<ExecReport | null>(null);
  /** Progress and Stop for the running execute. */
  readonly exec = new ExecTracker();
  error = $state<string | null>(null);

  /** Id of the scan in flight, learnt from its progress events. */
  private runningScanId: string | null = null;

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

  /** The active mode's scan id and recommendations. */
  private get active(): { scanId: string; recommendations: Recommendation[] } | null {
    return this.mode === "images" ? this.imageResult : this.result;
  }

  /** Bytes the current selection would reclaim. */
  get selectedBytes(): number {
    return (this.active?.recommendations ?? [])
      .filter((rec) => this.selected.has(rec.id))
      .reduce((total, rec) => total + rec.savingsBytes, 0);
  }

  /**
   * A single folder to scan instead of the saved roots.
   *
   * Set by the treemap's "Find duplicates here" cross-link, which is the glue
   * module spec 04 says not to skip: you see a fat folder on the map and ask
   * this module about that folder, not about your whole profile.
   */
  pinnedRoot = $state<string | null>(null);

  /** Run a scan. */
  async scan(): Promise<void> {
    await this.runScan(async () => {
      if (this.mode === "images") {
        // Nothing preselected: a look-alike is a judgement, not a fact.
        this.imageResult = await startSimilarImageScan(this.roots());
        return;
      }
      this.result = await startDuplicateScan(this.roots());
      this.selected = new Set(this.folderDefaults(this.result.folderGroups));
    });
  }

  private roots(): string[] | undefined {
    return this.pinnedRoot === null ? undefined : [this.pinnedRoot];
  }

  private async runScan(run: () => Promise<void>): Promise<void> {
    this.step = "scanning";
    this.error = null;
    this.report = null;
    this.itemsScanned = 0;
    this.pct = null;
    this.currentPath = null;
    this.stopping = false;
    this.runningScanId = null;
    this.selected = new Set();
    this.keeperOverrides = new Map();
    this.result = null;
    this.imageResult = null;
    try {
      await this.attachProgress();
      await run();
      this.step = "review";
    } catch (error: unknown) {
      this.error = message(error);
      this.step = "idle";
    } finally {
      this.detachProgress();
      this.runningScanId = null;
      this.stopping = false;
    }
  }

  /**
   * Ask the running scan to stop. The id comes from the scan's own progress
   * events, never from the previous result; a Stop pressed before the first
   * event is sent as soon as that event names the scan.
   */
  async stop(): Promise<void> {
    this.stopping = true;
    if (this.runningScanId) {
      await cancelScan(this.runningScanId);
    }
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
    // Folder-group picks are their own judgement; a file-set bulk action
    // leaves them alone.
    const folderIds = new Set(this.folderGroups.flatMap(folderRecIds));
    const selected = new Set(
      [...this.selected].filter((id) => folderIds.has(id)),
    );
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

  /** Duplicate-folder groups from the last scan. */
  get folderGroups(): FolderGroup[] {
    return this.result?.folderGroups ?? [];
  }

  /** Quarantine recommendations of every non-kept, unprotected folder. */
  private folderDefaults(groups: FolderGroup[]): string[] {
    const quarantine = new Set(
      (this.result?.recommendations ?? [])
        .filter((rec) => rec.action === "quarantine")
        .map((rec) => rec.id),
    );
    return groups
      .flatMap((group) => group.members)
      .filter((member) => !member.keeper && !member.protected)
      .flatMap((member) => member.recommendationIds)
      .filter((id) => quarantine.has(id));
  }

  /** Whether every recommendation of a folder copy is selected. */
  isFolderSelected(recommendationIds: string[]): boolean {
    return (
      recommendationIds.length > 0 &&
      recommendationIds.every((id) => this.selected.has(id))
    );
  }

  /** Toggle every recommendation of one folder copy together. */
  toggleFolder(recommendationIds: string[], next: boolean): void {
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
   * Change a group's keep rule. The backend re-analyses the group and swaps
   * its recommendations; the old ones leave the selection, the new non-kept
   * copies start selected as after a scan.
   */
  async setKeepRule(group: FolderGroup, rule: KeepRule): Promise<void> {
    const scan = this.result;
    if (!scan) {
      return;
    }
    this.error = null;
    try {
      const update = await setFolderKeepRule(scan.scanId, group.id, rule);
      const replaced = new Set(update.replaced);
      const result: DuplicateScanResult = {
        ...scan,
        folderGroups: scan.folderGroups.map((existing) =>
          existing.id === group.id ? update.group : existing,
        ),
        recommendations: [
          ...scan.recommendations.filter((rec) => !replaced.has(rec.id)),
          ...update.recommendations,
        ],
      };
      this.result = result;
      const selected = new Set(
        [...this.selected].filter((id) => !replaced.has(id)),
      );
      for (const id of this.folderDefaults([update.group])) {
        selected.add(id);
      }
      this.selected = selected;
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  /** Whether a look-alike image may be ticked: never the keeper or a protected one. */
  isImageSelectable(member: ImageMember): boolean {
    if (member.keeper || member.protected) {
      return false;
    }
    const ids = new Set(member.recommendationIds);
    return (this.imageResult?.recommendations ?? []).some(
      (rec) => ids.has(rec.id) && rec.action === "quarantine",
    );
  }

  /**
   * Keep a different image of a look-alike group. The backend re-analyses the
   * group; the old recommendations leave the selection and nothing new is
   * ticked, so the image just chosen can never end up selected.
   */
  async chooseImageKeeper(group: ImageGroup, member: ImageMember): Promise<void> {
    const scan = this.imageResult;
    if (!scan) {
      return;
    }
    this.error = null;
    try {
      const update = await setSimilarImageKeeper(scan.scanId, group.id, member.path);
      const replaced = new Set(update.replaced);
      this.imageResult = {
        ...scan,
        groups: scan.groups.map((existing) =>
          existing.id === group.id ? update.group : existing,
        ),
        recommendations: [
          ...scan.recommendations.filter((rec) => !replaced.has(rec.id)),
          ...update.recommendations,
        ],
      };
      this.selected = new Set(
        [...this.selected].filter((id) => !replaced.has(id)),
      );
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  /** Hide a group now and from future scans. */
  async ignoreGroup(group: FolderGroup): Promise<void> {
    const scan = this.result;
    if (!scan) {
      return;
    }
    this.error = null;
    try {
      await ignoreFolderGroup(group.id);
      const dropped = new Set(folderRecIds(group));
      this.result = {
        ...scan,
        folderGroups: scan.folderGroups.filter(
          (existing) => existing.id !== group.id,
        ),
      };
      this.selected = new Set(
        [...this.selected].filter((id) => !dropped.has(id)),
      );
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  /** Apply the selection. Duplicates never empty the recycle bin. */
  async execute(): Promise<void> {
    const scan = this.active;
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
    this.imageResult = null;
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
      const isFirstEvent = this.runningScanId === null;
      this.runningScanId = payload.scanId;
      if (isFirstEvent && this.stopping) {
        void cancelScan(payload.scanId);
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
