/**
 * Tweaks state (K-018).
 *
 * Applying goes through `execute`, so it passes the executor's gates and lands
 * in the audit log. Undo is the reversal and has its own command, exactly as
 * re-enabling a startup entry does: it puts back the value recorded before the
 * write, so there is no recommendation to re-verify.
 */
import { execute, tweaksScan, tweaksUndo } from "$lib/api";
import { t } from "$lib/i18n";
import { isApiError, type Recommendation, type TweakState, type TweaksScanResult } from "$lib/types";

import { itemNote } from "./execNotes";

/** Lifecycle of the read. */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

class TweaksStore {
  /** Every catalogue tweak with its live value. */
  tweaks = $state<TweakState[]>([]);
  /** Where the read is. */
  status = $state<LoadStatus>("idle");
  /** Message of the last failure. */
  error = $state<string | null>(null);
  /** Id of the tweak currently being changed, so its button can say so. */
  busy = $state<string | null>(null);
  /** What the last change did, in words. */
  lastReport = $state<string | null>(null);

  /** Scan id the apply action needs. */
  private scanId: string | null = null;
  /** Recommendation per tweak id, built by Rust at scan time. */
  private byTweak = new Map<string, Recommendation>();

  #loaded = false;
  #inflight: Promise<void> | null = null;

  /** The recommendation that applies `tweak`, or `null` when there is none. */
  recommendationOf(tweak: TweakState): Recommendation | null {
    return this.byTweak.get(tweak.id) ?? null;
  }

  /** Read, unless it has already been done. */
  load(): Promise<void> {
    if (this.#loaded) {
      return Promise.resolve();
    }
    return this.reload();
  }

  /** Share refreshes; a mutation performs its own fresh read. */
  reload(): Promise<void> {
    if (this.busy !== null) return Promise.resolve();
    this.#inflight ??= this.#doReload().finally(() => {
      this.#inflight = null;
    });
    return this.#inflight;
  }

  async #doReload(): Promise<void> {
    this.status = "loading";
    this.error = null;
    try {
      this.apply(await tweaksScan());
      this.status = "ready";
      this.#loaded = true;
    } catch (error: unknown) {
      this.tweaks = [];
      this.error = message(error);
      this.status = "error";
      this.#loaded = false;
    }
  }

  private apply(result: TweaksScanResult): void {
    this.scanId = result.scanId;
    this.tweaks = result.tweaks;
    this.byTweak = new Map();
    for (const recommendation of result.recommendations) {
      if (recommendation.justification.kind === "tweak") {
        this.byTweak.set(recommendation.justification.tweakId, recommendation);
      }
    }
  }

  /** Change a setting, through the executor. */
  async applyTweak(tweak: TweakState): Promise<void> {
    if (this.busy !== null || this.status === "loading") return;
    const recommendation = this.byTweak.get(tweak.id);
    if (this.scanId === null || !recommendation) {
      this.error = t("exec.settingStale");
      return;
    }
    this.busy = tweak.id;
    this.error = null;
    try {
      const report = await execute(this.scanId, [recommendation.id], false);
      const item = report.items[0];
      this.lastReport = item ? itemNote(item, t("exec.needsAdmin")) : null;
      await this.#doReload();
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = null;
    }
  }

  /** Put back the value that was there before `applyTweak`. */
  async undo(tweak: TweakState, done: string): Promise<void> {
    if (this.busy !== null || this.status === "loading") return;
    this.busy = tweak.id;
    this.error = null;
    try {
      await tweaksUndo(tweak.id);
      this.lastReport = done;
      await this.#doReload();
    } catch (error: unknown) {
      this.error = message(error);
    } finally {
      this.busy = null;
    }
  }
}

/** Shared tweaks state. */
export const tweaksStore = new TweaksStore();
