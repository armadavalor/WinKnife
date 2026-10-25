/**
 * Sidebar category tabs. Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import { categoryOf, groupByCategory, pickCategory } from "./categories.ts";

const tools = [
  { id: "cleaner", category: "cleanup" },
  { id: "network", category: "network" },
  { id: "duplicates", category: "cleanup" },
  { id: "mystery", category: "brand-new" },
  { id: "legacy" },
];

test("categoryOf falls back to other for unknown or missing categories", () => {
  assert.equal(categoryOf({ category: "files" }), "files");
  assert.equal(categoryOf({ category: "brand-new" }), "other");
  assert.equal(categoryOf({}), "other");
});

test("groupByCategory keeps tab order, registry order, and drops empty tabs", () => {
  const groups = groupByCategory(tools);
  assert.deepEqual(
    groups.map((g) => [g.category, g.modules.map((m) => m.id)]),
    [
      ["cleanup", ["cleaner", "duplicates"]],
      ["network", ["network"]],
      ["other", ["mystery", "legacy"]],
    ],
  );
});

test("pickCategory keeps the wanted tab only while it has tools", () => {
  const groups = groupByCategory(tools);
  assert.equal(pickCategory(groups, "network"), "network");
  assert.equal(pickCategory(groups, "power"), "cleanup");
  assert.equal(pickCategory(groups, null), "cleanup");
  assert.equal(pickCategory([], "network"), null);
});
