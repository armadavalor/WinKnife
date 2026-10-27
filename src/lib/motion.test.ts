/**
 * The animations preference. Run with `npm run test:ui`.
 *
 * Three values: `system` (default) follows the OS reduced-motion setting, `on`
 * ignores it, `off` is always off. A legacy stored "on" meant "follow the OS",
 * so it migrates to `system`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  animationsEnabled,
  isAnimationPref,
  migrateLegacyPref,
  parseAnimationPref,
} from "./motion.ts";

test("system, on and off are the preferences", () => {
  for (const ok of ["system", "on", "off"]) assert.equal(isAnimationPref(ok), true);
  assert.equal(isAnimationPref("auto"), false);
  assert.equal(isAnimationPref(null), false);
  assert.equal(isAnimationPref(1), false);
});

test("a missing or unknown stored value means system", () => {
  assert.equal(parseAnimationPref(null), "system");
  assert.equal(parseAnimationPref("garbage"), "system");
  assert.equal(parseAnimationPref("on"), "on");
  assert.equal(parseAnimationPref("off"), "off");
});

test("system follows the OS; on ignores it; off never runs", () => {
  assert.equal(animationsEnabled("system", false), true);
  assert.equal(animationsEnabled("system", true), false);
  assert.equal(animationsEnabled("on", false), true);
  assert.equal(animationsEnabled("on", true), true);
  assert.equal(animationsEnabled("off", false), false);
  assert.equal(animationsEnabled("off", true), false);
});

test("the legacy 'on' becomes system, the legacy 'off' stays off", () => {
  assert.equal(migrateLegacyPref("on"), "system");
  assert.equal(migrateLegacyPref("off"), "off");
  assert.equal(migrateLegacyPref(null), "system");
  assert.equal(migrateLegacyPref("garbage"), "system");
});
