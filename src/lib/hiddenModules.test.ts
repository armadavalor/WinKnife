/**
 * Tools hidden from the sidebar (issue #8). Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import { parseHiddenModules, toggleHidden, visibleModules } from "./hiddenModules.ts";

test("parseHiddenModules keeps string ids and drops everything else", () => {
  assert.deepEqual(parseHiddenModules('["treemap","startup"]'), ["treemap", "startup"]);
  assert.deepEqual(parseHiddenModules('["treemap",3,null,"treemap"]'), ["treemap"]);
  assert.deepEqual(parseHiddenModules(null), []);
  assert.deepEqual(parseHiddenModules("not json"), []);
  assert.deepEqual(parseHiddenModules('{"treemap":true}'), []);
});

test("toggleHidden adds and removes an id without touching the input", () => {
  const hidden = ["treemap"];
  assert.deepEqual(toggleHidden(hidden, "startup"), ["treemap", "startup"]);
  assert.deepEqual(toggleHidden(hidden, "treemap"), []);
  assert.deepEqual(hidden, ["treemap"]);
});

test("visibleModules keeps registry order and ignores unknown hidden ids", () => {
  const modules = [{ id: "cleaner" }, { id: "treemap" }, { id: "startup" }];
  assert.deepEqual(visibleModules(modules, ["treemap", "gone"]), [
    { id: "cleaner" },
    { id: "startup" },
  ]);
  assert.deepEqual(visibleModules(modules, []), modules);
});
