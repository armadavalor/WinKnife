/** Panel translucency clamping. Run with `npm run test:ui`. */
import assert from "node:assert/strict";
import { test } from "node:test";
import { clampOpacity, parseOpacity, PANEL_DEFAULT } from "./panelOpacity.ts";

test("values clamp to the readable range", () => {
  assert.equal(clampOpacity(0, 60), 30);
  assert.equal(clampOpacity(500, 60), 100);
  assert.equal(clampOpacity(55.4, 60), 55);
});

test("garbage falls back", () => {
  assert.equal(parseOpacity(null, PANEL_DEFAULT), 60);
  assert.equal(parseOpacity("abc", PANEL_DEFAULT), 60);
  assert.equal(parseOpacity("80", PANEL_DEFAULT), 80);
});
