/**
 * Pure helpers behind the quarantine drawer, kept out of the runes store so
 * they can be tested with `npm run test:ui`.
 */
import type { PurgeFailure, PurgeReport, QuarantineBatch, QuarantineItem } from "../types.ts";

/** Items whose original path contains `query`, ignoring case. */
export function filterItems(items: QuarantineItem[], query: string): QuarantineItem[] {
  const needle = query.trim().toLowerCase();
  if (!needle) {
    return items;
  }
  return items.filter((item) => item.originalPath.toLowerCase().includes(needle));
}

/** What the drawer says after "Remove for good now". */
export type PurgeOutcome =
  | { kind: "nothingDue"; nextDue: string | null }
  | { kind: "done"; count: number; bytes: number; batches: number; failures: PurgeFailureCount[] };

export interface PurgeFailureCount {
  reason: PurgeFailure;
  count: number;
}

/**
 * Turn a purge report into the line the drawer shows. A purge that removed
 * nothing used to show nothing at all, so pressing the button looked broken
 * (2026-10-25): it now says when the earliest batch is due instead.
 */
export function purgeOutcome(report: PurgeReport, batches: QuarantineBatch[]): PurgeOutcome {
  if (report.purgedItems === 0 && report.failedItems.length === 0) {
    const due = batches.map((b) => b.purgeAfter).sort();
    return { kind: "nothingDue", nextDue: due[0] ?? null };
  }
  const counts = new Map<PurgeFailure, number>();
  for (const id of report.failedItems) {
    const reason = report.failureReasons?.[id] ?? "other";
    counts.set(reason, (counts.get(reason) ?? 0) + 1);
  }
  return {
    kind: "done",
    count: report.purgedItems,
    bytes: report.freedBytes,
    batches: report.affectedBatches.length,
    failures: [...counts].map(([reason, count]) => ({ reason, count })),
  };
}
