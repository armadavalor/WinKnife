/**
 * Which execute and restore results the user must be told about. Run with
 * `npm run test:ui`.
 *
 * A move that crosses drives can succeed and still leave part of a folder
 * behind, or keep only the moved copy. Those warnings used to stop at the log;
 * they must reach the screen (adversarial review follow-up A).
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import type { ExecItemResult, MoveWarning } from "../types.ts";
import {
  CANCELLED_REASON,
  isRestoreRefused,
  itemsToReport,
  refusedHeld,
  itemNote,
  stopSummary,
  warningsNote,
  warningsOf,
} from "./execNotes.ts";

const LEFT: MoveWarning = {
  path: "D:\\tree\\b.txt",
  reason: "part of the folder stayed where it was moved from",
};

function item(
  findingId: string,
  outcome: ExecItemResult["outcome"],
  warnings?: MoveWarning[],
): ExecItemResult {
  return {
    recommendationId: `r-${findingId}`,
    findingId,
    path: `C:\\${findingId}`,
    outcome,
    reclaimedBytes: 0,
    ...(warnings ? { warnings } : {}),
  };
}

test("a clean success is not reported", () => {
  const items = [item("a", { status: "succeeded" })];
  assert.deepEqual(itemsToReport(items), []);
});

test("a success that left something behind is reported", () => {
  const warned = item("a", { status: "succeeded" }, [LEFT]);
  assert.deepEqual(itemsToReport([warned]), [warned]);
});

test("skips and failures are still reported", () => {
  const skipped = item("s", { status: "skipped", reason: "protected" });
  const failed = item("f", { status: "failed", error: "busy" });
  assert.deepEqual(itemsToReport([skipped, failed]), [skipped, failed]);
});

test("items a stop left untouched are summed up, not listed one by one", () => {
  const moved = item("m", { status: "succeeded" });
  const stopped = item("c", { status: "skipped", reason: CANCELLED_REASON });
  const protectedItem = item("p", { status: "skipped", reason: "protected" });
  const items = [moved, stopped, protectedItem];
  assert.deepEqual(itemsToReport(items), [protectedItem]);
  assert.deepEqual(stopSummary(items), { moved: 1, left: 1 });
});

test("a run nobody stopped has no stop summary", () => {
  const items = [
    item("m", { status: "succeeded" }),
    item("p", { status: "skipped", reason: "protected" }),
  ];
  assert.equal(stopSummary(items), null);
});

test("results from before warnings existed read as having none", () => {
  assert.deepEqual(warningsOf({}), []);
  assert.deepEqual(warningsOf({ warnings: [LEFT] }), [LEFT]);
});

test("a single result's note names what went wrong", () => {
  assert.equal(itemNote(item("a", { status: "succeeded" })), null);
  assert.equal(
    itemNote(item("s", { status: "skipped", reason: "protected" })),
    "protected",
  );
  assert.equal(itemNote(item("f", { status: "failed", error: "busy" })), "busy");
  assert.equal(
    itemNote(item("a", { status: "succeeded" }, [LEFT])),
    `${LEFT.path}: ${LEFT.reason}`,
  );
});

test("a bare warning list, as a re-enable returns it, becomes one note", () => {
  assert.equal(warningsNote([]), null);
  assert.equal(warningsNote([LEFT]), `${LEFT.path}: ${LEFT.reason}`);
  assert.equal(
    warningsNote([LEFT, LEFT]),
    `${LEFT.path}: ${LEFT.reason}; ${LEFT.path}: ${LEFT.reason}`,
  );
});

test("an item whose encrypted restore was refused is marked as kept", () => {
  // Decision b (2026-10-12): such an item is kept until the user removes it.
  assert.equal(isRestoreRefused({ restoreRefusedAt: "2026-10-12T00:00:00.000Z" }), true);
  assert.equal(isRestoreRefused({ restoreRefusedAt: null }), false);
  // A ledger written before the marker existed sends nothing.
  assert.equal(isRestoreRefused({}), false);
});

test("the drawer explains refused items whenever any are held", () => {
  // Round 4: they are outside the size cap, so the user is told what they
  // hold whatever their size, not only once they fill the cap.
  const stats = { itemCount: 3, totalBytes: 40, unreadableRows: 0 };
  assert.equal(refusedHeld({ ...stats, refusedItems: 1, refusedBytes: 1, capBytes: 25 }), true);
  assert.equal(refusedHeld({ ...stats, refusedItems: 2, refusedBytes: 20, capBytes: 15 }), true);
  assert.equal(refusedHeld({ ...stats, refusedItems: 0, refusedBytes: 0, capBytes: 25 }), false);
  // A backend from before round 3 sends none of the fields.
  assert.equal(refusedHeld(stats), false);
});
