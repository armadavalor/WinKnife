/**
 * Which look a progress bar takes. Run with `npm run test:ui`.
 *
 * A bar that stands still reads as frozen (Polish item 5), so a running bar
 * always carries some motion; a static gauge like disk capacity carries none.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import { clampPct, progressMode } from "./mode.ts";

test("an unknown total is indeterminate", () => {
  assert.equal(progressMode(null, true), "indeterminate");
});

test("a known total on running work moves", () => {
  assert.equal(progressMode(0, true), "running");
  assert.equal(progressMode(42, true), "running");
});

test("a finished bar stops moving", () => {
  assert.equal(progressMode(100, true), "static");
});

test("a gauge that is not work in progress never moves", () => {
  assert.equal(progressMode(63, false), "static");
});

test("percent is clamped to 0-100", () => {
  assert.equal(clampPct(null), null);
  assert.equal(clampPct(-5), 0);
  assert.equal(clampPct(140), 100);
  assert.equal(clampPct(Number.NaN), 0);
});
