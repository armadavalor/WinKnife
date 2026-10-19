/**
 * Treemap state: which drive, which folder, and what the last scan found.
 *
 * The picture is built from two backend calls that do very different amounts of
 * work — `treemapScan` walks a whole drive once, `treemapChildren` reads one
 * folder — so the store keeps them apart: `scanning` drives the progress line,
 * `zooming` drives a much quieter one, and neither is allowed to look like the
 * other.
 */
import {
  cancelScan,
  onScanProgress,
  treemapCached,
  treemapChildren,
  treemapDrives,
  treemapScan,
} from "$lib/api";
import {
  isApiError,
  type CachedScan,
  type DriveInfo,
  type TreeLevel,
} from "$lib/types";

/** Lifecycle of the drive list. */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class TreemapStore {
  /** Drives that can be mapped. */
  drives = $state<DriveInfo[]>([]);
  /** Which one is selected, as its root path. */
  selected = $state<string | null>(null);
  /** Where the drive list is. */
  status = $state<LoadStatus>("idle");
  /** Message of the last failure. */
  error = $state<string | null>(null);

  /** Header of the cached scan for the selected drive, if any. */
  cached = $state<CachedScan | null>(null);
  /** The folder currently drawn. */
  level = $state<TreeLevel | null>(null);

  /** Whether a whole-drive walk is running. */
  scanning = $state(false);
  /** Whether one folder is being fetched — a much smaller wait. */
  zooming = $state(false);
  /** Id of the running walk, so it can be stopped. */
  scanId = $state<string | null>(null);
  /** Stop was pressed; sent as soon as an event names the walk. */
  stopping = $state(false);
  /** Files seen so far, for the progress line. */
  scanned = $state(0);
  /** Where the walk is right now. */
  currentPath = $state<string | null>(null);
  /** Set when the last walk was stopped early. */
  partial = $state(false);
  /** Reparse points the last walk drew but did not count. */
  links = $state<string[]>([]);
  /** Folders the last walk could not read. */
  unreadable = $state<string[]>([]);

  /** The drive record for the current selection. */
  get drive(): DriveInfo | null {
    return this.drives.find((d) => d.root === this.selected) ?? null;
  }

  /**
   * The drive-list fetch currently running, if one is.
   *
   * Plain field, never `$state`: the guard it replaced read `status`, which
   * subscribed any effect that called `loadDrives()` to `status` and turned the
   * guard itself into a re-entry loop.
   */
  #inflight: Promise<void> | null = null;

  /** Load the drive list and pick a sensible default. */
  loadDrives(): Promise<void> {
    this.#inflight ??= this.#doLoadDrives().finally(() => {
      this.#inflight = null;
    });
    return this.#inflight;
  }

  async #doLoadDrives(): Promise<void> {
    this.status = "loading";
    this.error = null;
    try {
      this.drives = await treemapDrives();
      this.status = "ready";
      if (this.selected === null && this.drives.length > 0) {
        await this.select(this.drives[0].root);
      }
    } catch (error: unknown) {
      this.drives = [];
      this.error = message(error);
      this.status = "error";
    }
  }

  /** Switch drive, showing whatever is cached for it without re-walking. */
  async select(root: string): Promise<void> {
    this.selected = root;
    this.level = null;
    this.partial = false;
    this.links = [];
    this.unreadable = [];
    this.error = null;
    try {
      this.cached = await treemapCached(root);
      if (this.cached) {
        this.level = await treemapChildren(root, root);
      }
    } catch (error: unknown) {
      this.error = message(error);
    }
  }

  /** Walk the selected drive. */
  async scan(): Promise<void> {
    const root = this.selected;
    if (root === null || this.scanning) {
      return;
    }
    this.scanning = true;
    this.scanId = null;
    this.stopping = false;
    this.scanned = 0;
    this.currentPath = null;
    this.error = null;
    try {
      const result = await treemapScan(root);
      this.scanId = result.scanId;
      this.cached = result.summary;
      this.level = result.level;
      this.partial = result.cancelled;
      this.links = result.links;
      this.unreadable = result.unreadable;
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.scanning = false;
      this.scanId = null;
      this.stopping = false;
      this.currentPath = null;
    }
  }

  /**
   * Ask the running walk to stop. Partial results are still shown. The id
   * comes from progress events, so a Stop pressed before the first one is
   * sent as soon as that event names the walk.
   */
  async stop(): Promise<void> {
    if (!this.scanning) {
      return;
    }
    this.stopping = true;
    if (this.scanId !== null) {
      await cancelScan(this.scanId);
    }
  }

  /** Draw a different folder. */
  async open(path: string): Promise<void> {
    const root = this.selected;
    if (root === null || this.zooming) {
      return;
    }
    this.zooming = true;
    this.error = null;
    try {
      this.level = await treemapChildren(root, path);
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.zooming = false;
    }
  }

  /**
   * Keep the progress line live.
   *
   * Filtered to this module: a duplicate scan started from another route emits
   * on the same channel, and a treemap progress line reporting someone else's
   * file count is worse than no line at all.
   */
  async listen(): Promise<() => void> {
    return onScanProgress((payload) => {
      // A late event from a finished walk must not bring its id back.
      if (payload.module !== "treemap" || !this.scanning) {
        return;
      }
      const isFirstEvent = this.scanId === null;
      this.scanId = payload.scanId;
      if (isFirstEvent && this.stopping) {
        void cancelScan(payload.scanId);
      }
      this.scanned = payload.itemsScanned;
      this.currentPath = payload.currentPath;
    });
  }
}

/** Shared treemap state. */
export const treemapStore = new TreemapStore();
