/**
 * Which execute and restore results the user must be told about.
 *
 * Kept free of Svelte and `$lib` imports so `node --test` can run it directly.
 */
import type {
  ApiError,
  ExecItemResult,
  MoveWarning,
  QuarantineItem,
  QuarantineStats,
} from "../types.ts";

/** An item's warnings; results from before warnings existed have none. */
export function warningsOf(result: {
  warnings?: readonly MoveWarning[];
}): readonly MoveWarning[] {
  return result.warnings ?? [];
}

/**
 * The items worth a line under a finished run: anything that did not succeed,
 * and any success that did not go entirely as planned (adversarial review
 * follow-up A).
 */
export function itemsToReport(
  items: readonly ExecItemResult[],
): ExecItemResult[] {
  return items.filter(
    (item) =>
      item.outcome.status !== "succeeded" || warningsOf(item).length > 0,
  );
}

/** One line for a single result, or null when there is nothing to say. */
export function itemNote(item: ExecItemResult): string | null {
  if (item.outcome.status === "skipped") return item.outcome.reason;
  if (item.outcome.status === "failed") return item.outcome.error;
  return warningsNote(warningsOf(item));
}

/**
 * One line for a list of warnings, or null when there are none. A re-enabled
 * startup shortcut returns its warnings bare, with no item around them
 * (round-2 review).
 */
export function warningsNote(warnings: readonly MoveWarning[]): string | null {
  if (warnings.length === 0) return null;
  return warnings.map((w) => `${w.path}: ${w.reason}`).join("; ");
}

/**
 * Whether a held item's restore was refused because it is encrypted. Such an
 * item is kept until the user removes it (decision b, 2026-10-12); a ledger
 * from before the marker existed sends nothing.
 */
export function isRestoreRefused(
  item: Pick<QuarantineItem, "restoreRefusedAt">,
): boolean {
  return typeof item.restoreRefusedAt === "string";
}

/**
 * Whether any items kept after a refused restore are held. They are outside
 * the size cap and only the user removes them, so the drawer says what they
 * hold whenever there are any (round 4). A backend from before round 3 sends
 * none of the fields.
 */
export function refusedHeld(stats: QuarantineStats): boolean {
  return (stats.refusedItems ?? 0) > 0 || (stats.refusedBytes ?? 0) > 0;
}

/** Error codes with which the backend refuses to remove an item by hand. */
const NOT_ELIGIBLE_CODES: readonly string[] = ["unknown_quarantine_item", "quarantine"];

/**
 * The backend's `{ code, message }` error, or null. A value import of
 * `isApiError` would need a `.ts` path that svelte-check rejects, and this
 * file keeps to type-only imports so `node --test` can run it.
 */
function asApiError(error: unknown): Pick<ApiError, "code" | "message"> | null {
  if (typeof error !== "object" || error === null) return null;
  const { code, message } = error as Record<string, unknown>;
  return typeof code === "string" && typeof message === "string" ? { code, message } : null;
}

/**
 * Why removing one refused item by hand did not work, or null when it went.
 *
 * - `stuck`: the backend ran but could not delete the file, so the item is
 *   still held and still marked.
 * - `notEligible`: the backend refused because the item is no longer held or
 *   is not marked (a stale list); nothing can be said about where it is.
 * - `error`: anything else, shown as it came (round 4).
 */
export type RemovalFailure =
  | { kind: "stuck" }
  | { kind: "notEligible" }
  | { kind: "error"; error: string };

export function removalFailure(
  itemId: string,
  result: { failedItems: readonly string[] } | { error: unknown },
): RemovalFailure | null {
  if ("error" in result) {
    const api = asApiError(result.error);
    if (api) {
      return NOT_ELIGIBLE_CODES.includes(api.code)
        ? { kind: "notEligible" }
        : { kind: "error", error: api.message };
    }
    return { kind: "error", error: String(result.error) };
  }
  return result.failedItems.includes(itemId) ? { kind: "stuck" } : null;
}
