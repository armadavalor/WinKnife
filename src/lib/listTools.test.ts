import assert from "node:assert/strict";
import { test } from "node:test";

import {
  deselectVisible,
  invertVisible,
  matchesFilter,
  parseCollapsed,
  selectVisible,
  sharedText,
  sortRows,
} from "./listTools.ts";

const rows = [
  { id: "a", selectable: true },
  { id: "b", selectable: false },
  { id: "c", selectable: true },
];

test("filter is case-insensitive over any field and blank matches all", () => {
  assert.ok(matchesFilter("", "x"));
  assert.ok(matchesFilter("  TEMP ", "C:/Users/a/temp/f.tmp"));
  assert.ok(matchesFilter("cache", "C:/x", "Browser Cache"));
  assert.ok(!matchesFilter("zzz", "C:/x", undefined));
});

test("select-visible never ticks a non-selectable row", () => {
  const next = selectVisible(new Set(["hidden"]), rows);
  assert.deepEqual([...next].sort(), ["a", "c", "hidden"]);
});

test("deselect-visible keeps hidden rows selected", () => {
  const next = deselectVisible(new Set(["a", "hidden"]), rows);
  assert.deepEqual([...next], ["hidden"]);
});

test("invert flips selectable visible rows only", () => {
  const next = invertVisible(new Set(["a", "hidden"]), rows);
  assert.deepEqual([...next].sort(), ["c", "hidden"]);
});

test("sort by size desc and name asc without mutating", () => {
  const input = [{ n: "b", s: 1 }, { n: "a", s: 9 }];
  const bySize = sortRows(input, "size", (r) => r.s, (r) => r.n);
  const byName = sortRows(input, "name", (r) => r.s, (r) => r.n);
  assert.equal(bySize[0].n, "a");
  assert.equal(byName[0].n, "a");
  assert.equal(input[0].n, "b");
});

test("collapse parsing survives garbage", () => {
  assert.deepEqual([...parseCollapsed('["x",1]')], ["x"]);
  assert.equal(parseCollapsed("{oops").size, 0);
  assert.equal(parseCollapsed(null).size, 0);
});

test("sharedText returns the common text only", () => {
  assert.equal(sharedText(["r", "r"]), "r");
  assert.equal(sharedText(["r", "q"]), null);
  assert.equal(sharedText([]), null);
});
