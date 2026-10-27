/**
 * Which rows start ticked after a scan. Run with `npm run test:ui`.
 *
 * The recycle bin is the one cleaner action that is not reversible, so it must
 * never be chosen for the user: they tick it themselves, then confirm the
 * warning (Phase 4 release audit).
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import type { Finding, Recommendation } from "../types.ts";
import { initialSelection } from "./selection.ts";

function finding(id: string, kind: Finding["kind"]): Finding {
  return { id, kind } as Finding;
}

function rec(
  id: string,
  findingId: string,
  risk: Recommendation["risk"],
  action: Recommendation["action"] = "quarantine" as Recommendation["action"],
): Recommendation {
  return { id, findingIds: [findingId], risk, action } as Recommendation;
}

test("green actionable rows start ticked, yellow and info-only do not", () => {
  const findings = [finding("f1", "file"), finding("f2", "file"), finding("f3", "directory")];
  const recs = [
    rec("r1", "f1", "safe"),
    rec("r2", "f2", "review"),
    rec("r3", "f3", "safe", "infoOnly"),
  ];
  assert.deepEqual([...initialSelection(recs, findings)], ["r1"]);
});

test("a green recycle bin row never starts ticked", () => {
  const findings = [finding("f1", "file"), finding("bin", "recycleBinItem")];
  const recs = [rec("r1", "f1", "safe"), rec("rb", "bin", "safe")];
  assert.deepEqual([...initialSelection(recs, findings)], ["r1"]);
});

test("a recommendation whose finding is missing is not selected", () => {
  assert.equal(initialSelection([rec("r1", "gone", "safe")], []).size, 0);
});
