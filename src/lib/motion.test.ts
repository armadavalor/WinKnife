/**
 * The animations switch (Polish item 8). Run with `npm run test:ui`.
 *
 * On by default; one toggle turns it off; the system's reduced-motion setting
 * turns it off whatever the toggle says.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import { animationsEnabled, isAnimationPref, parseAnimationPref } from "./motion.ts";

test("only 'on' and 'off' are preferences", () => {
  assert.equal(isAnimationPref("on"), true);
  assert.equal(isAnimationPref("off"), true);
  assert.equal(isAnimationPref("auto"), false);
  assert.equal(isAnimationPref(null), false);
  assert.equal(isAnimationPref(1), false);
});

test("a missing or unknown stored value means on", () => {
  assert.equal(parseAnimationPref(null), "on");
  assert.equal(parseAnimationPref("garbage"), "on");
  assert.equal(parseAnimationPref("off"), "off");
});

test("animations run only when the toggle is on and the system allows motion", () => {
  assert.equal(animationsEnabled("on", false), true);
  assert.equal(animationsEnabled("off", false), false);
  assert.equal(animationsEnabled("on", true), false);
  assert.equal(animationsEnabled("off", true), false);
});
