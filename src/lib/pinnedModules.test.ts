/** Dashboard pins. Run with `npm run test:ui`. */
import assert from "node:assert/strict";
import { test } from "node:test";
import { DEFAULT_PINNED, parsePinnedModules, pinnedModules, togglePinned } from "./pinnedModules.ts";

test("nothing or garbage stored gives the starter set; an empty list stays empty", () => {
  assert.deepEqual(parsePinnedModules(null), DEFAULT_PINNED);
  assert.deepEqual(parsePinnedModules("{"), DEFAULT_PINNED);
  assert.deepEqual(parsePinnedModules('{"a":1}'), DEFAULT_PINNED);
  assert.deepEqual(parsePinnedModules("[]"), []);
});

test("stored ids are kept in order, deduped, strings only", () => {
  assert.deepEqual(parsePinnedModules('["b","a",3,"b"]'), ["b", "a"]);
});

test("toggle appends when absent and removes when present", () => {
  assert.deepEqual(togglePinned(["a"], "b"), ["a", "b"]);
  assert.deepEqual(togglePinned(["a", "b"], "a"), ["b"]);
});

test("pinnedModules keeps pinned order and drops unknown ids", () => {
  const mods = [{ id: "a" }, { id: "b" }, { id: "c" }];
  assert.deepEqual(pinnedModules(mods, ["c", "x", "a"]), [{ id: "c" }, { id: "a" }]);
});
