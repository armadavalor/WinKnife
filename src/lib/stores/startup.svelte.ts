/**
 * Startup manager state.
 *
 * Turning something **off** goes through `execute`, so it passes the same four
 * gates and lands in the same audit log as every other change this app makes.
 * Turning it back **on** is the reversal and has its own command, exactly as
 * restoring from quarantine does — there is no recommendation to re-verify when
 * putting something back the way it was found.
 */
import { execute, startupEnable, startupScan } from "$lib/api";
import {
  isApiError,
  type Recommendation,
  type StartupEntry,
  type StartupScanResult,
} from "$lib/types";

/** Lifecycle of the enumeration. */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class StartupStore {
  /** Everything that runs at logon. */
  entries = $state<StartupEntry[]>([]);
  /** Sources that could not be read, so a partial list says it is one. */
  unreadable = $state<string[]>([]);
  /** Where the enumeration is. */
  status = $state<LoadStatus>("idle");
  /** Message of the last failure. */
  error = $state<string | null>(null);
  /** Id of the entry currently being changed, so its button can say so. */
  busy = $state<string | null>(null);
  /** What the last change did, in words. */
  lastReport = $state<string | null>(null);

  /** Scan id the disable action needs. */
  private scanId: string | null = null;
  /** Recommendation per entry id, built by Rust at scan time. */
  private byEntry = new Map<string, Recommendation>();

  /** How many entries currently run. */
  get enabledCount(): number {
    return this.entries.filter((entry) => entry.enabled).length;
  }

  /**
   * Non-reactive mirrors of "an enumeration succeeded" and "one is running".
   *
   * `load()` used to answer both questions by reading `status`, which subscribed
   * the calling effect to `status` and made every transition re-enter the scan.
   * Plain fields say the same thing without being observable.
   */
  #loaded = false;
  #inflight: Promise<void> | null = null;

  /** Enumerate, unless it has already been done. */
  load(): Promise<void> {
    if (this.#loaded) {
      return Promise.resolve();
    }
    return this.reload();
  }

  /** Share refreshes; a mutation performs its own fresh enumeration. */
  reload(): Promise<void> {
    if (this.busy !== null) return Promise.resolve();
    this.#inflight ??= this.#doReload().finally(() => {
      this.#inflight = null;
    });
    return this.#inflight;
  }

  /** Enumerate again, whatever the current state. */
  async #doReload(): Promise<void> {
    this.status = "loading";
    this.error = null;
    try {
      this.apply(await startupScan());
      this.status = "ready";
      this.#loaded = true;
    } catch (error: unknown) {
      this.entries = [];
      this.error = message(error);
      this.status = "error";
      this.#loaded = false;
    }
  }

  private apply(result: StartupScanResult): void {
    this.scanId = result.scanId;
    this.entries = result.scan.entries;
    this.unreadable = result.scan.unreadable;
    this.byEntry = new Map();
    // A recommendation names its finding, and each finding is one entry. Rust
    // emits them in the same order as the entries it could offer, so pairing is
    // by the finding id the entry's recommendation carries.
    for (const recommendation of result.recommendations) {
      const entry = this.entries.find(
        (candidate) =>
          candidate.enabled &&
          !candidate.os_owned &&
          recommendation.justification.kind === "startupEntry" &&
          recommendation.justification.key === candidate.key,
      );
      if (entry) {
        this.byEntry.set(entry.id, recommendation);
      }
    }
  }

  /** Turn an entry off, through the executor. */
  async disable(entry: StartupEntry): Promise<void> {
    if (this.busy !== null || this.status === "loading") return;
    const recommendation = this.byEntry.get(entry.id);
    if (this.scanId === null || !recommendation) {
      this.error = "This entry can no longer be changed; refresh the list.";
      return;
    }
    this.busy = entry.id;
    this.error = null;
    try {
      const report = await execute(this.scanId, [recommendation.id], false);
      const item = report.items[0];
      if (item && item.outcome.status !== "succeeded") {
        // A skip carries the reason Rust refused; showing it beats a silent
        // toggle that springs back.
        this.lastReport =
          item.outcome.status === "skipped"
            ? item.outcome.reason
            : item.outcome.error;
      } else {
        this.lastReport = null;
      }
      await this.#doReload();
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = null;
    }
  }

  /** Turn an entry back on. */
  async enable(entry: StartupEntry): Promise<void> {
    if (this.busy !== null || this.status === "loading") return;
    this.busy = entry.id;
    this.error = null;
    try {
      await startupEnable(entry.source, entry.key);
      this.lastReport = null;
      await this.#doReload();
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = null;
    }
  }
}

/** Shared startup state. */
export const startupStore = new StartupStore();
