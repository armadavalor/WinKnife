/**
 * Quarantine drawer helpers. Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import type { PurgeReport, QuarantineBatch, QuarantineItem } from "../types.ts";
import { filterItems, purgeOutcome } from "./quarantineView.ts";

function report(over: Partial<PurgeReport> = {}): PurgeReport {
  return { purgedItems: 0, freedBytes: 0, failedItems: [], affectedBatches: [], failureReasons: {}, ...over };
}

function batch(purgeAfter: string): QuarantineBatch {
  return { purgeAfter } as QuarantineBatch;
}

test("a purge that removed nothing says when the earliest batch is due", () => {
  const outcome = purgeOutcome(report(), [
    batch("2026-11-20T00:00:00Z"),
    batch("2026-11-02T00:00:00Z"),
  ]);
  assert.deepEqual(outcome, { kind: "nothingDue", nextDue: "2026-11-02T00:00:00Z" });
});

test("an empty quarantine has nothing due", () => {
  assert.deepEqual(purgeOutcome(report(), []), { kind: "nothingDue", nextDue: null });
});

test("failures are counted by reason, unknown ids fall back to other", () => {
  const outcome = purgeOutcome(
    report({
      purgedItems: 2,
      freedBytes: 10,
      affectedBatches: ["b"],
      failedItems: ["a", "b", "c"],
      failureReasons: { a: "locked", b: "locked" },
    }),
    [],
  );
  assert.deepEqual(outcome, {
    kind: "done",
    count: 2,
    bytes: 10,
    batches: 1,
    failures: [
      { reason: "locked", count: 2 },
      { reason: "other", count: 1 },
    ],
  });
});

test("only failures still counts as a result, not as nothing due", () => {
  const outcome = purgeOutcome(report({ failedItems: ["a"] }), []);
  assert.equal(outcome.kind, "done");
});

test("the filter matches the original path case-insensitively", () => {
  const items = [
    { originalPath: "C:\\Users\\a\\Temp\\Log.txt" },
    { originalPath: "C:\\Users\\a\\Downloads\\setup.exe" },
  ] as QuarantineItem[];
  assert.equal(filterItems(items, "  log ").length, 1);
  assert.equal(filterItems(items, "").length, 2);
});
