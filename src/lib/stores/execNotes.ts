/**
 * Which execute and restore results the user must be told about.
 *
 * Kept free of Svelte and `$lib` imports so `node --test` can run it directly.
 */
import type {
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
 * The reason the executor gives an item a Stop left untouched; must match
 * `core::executor::CANCELLED_REASON` (issue #39).
 */
export const CANCELLED_REASON = "cancelled by the user";

function wasStopped(item: ExecItemResult): boolean {
  return item.outcome.status === "skipped" && item.outcome.reason === CANCELLED_REASON;
}

/**
 * The items worth a line under a finished run: anything that did not succeed,
 * and any success that did not go entirely as planned (adversarial review
 * follow-up A). Items a Stop left untouched are summed up by
 * {@link stopSummary} instead: a stop early in a big batch would otherwise
 * list thousands of identical lines.
 */
export function itemsToReport(
  items: readonly ExecItemResult[],
): ExecItemResult[] {
  return items.filter(
    (item) =>
      !wasStopped(item) &&
      (item.outcome.status !== "succeeded" || warningsOf(item).length > 0),
  );
}

/**
 * How a stopped run ended: how many items moved and how many the stop left
 * where they were. Null when nobody stopped the run.
 */
export function stopSummary(
  items: readonly ExecItemResult[],
): { moved: number; left: number } | null {
  const left = items.filter(wasStopped).length;
  if (left === 0) return null;
  const moved = items.filter((item) => item.outcome.status === "succeeded").length;
  return { moved, left };
}

/** English default for `itemNote`; the stores pass the translated text (K-021). */
export const NEEDS_ADMIN_NOTE =
  "This needs administrator rights — use Restart as admin in the sidebar.";

/** One line for a single result, or null when there is nothing to say. */
export function itemNote(
  item: ExecItemResult,
  needsAdminNote: string = NEEDS_ADMIN_NOTE,
): string | null {
  if (item.outcome.status === "skipped") return item.outcome.reason;
  if (item.outcome.status === "failed") {
    return item.outcome.needsAdmin ? needsAdminNote : item.outcome.error;
  }
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
