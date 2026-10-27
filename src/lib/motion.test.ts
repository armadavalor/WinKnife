/**
 * The animations preference. Run with `npm run test:ui`.
 *
 * Three values: `on` (default) ignores the OS reduced-motion setting, `system`
 * follows it, `off` is always off. A legacy stored "on" meant "follow the OS",
 * so it migrates to `system`; nothing stored means the default.
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

test("a missing or unknown stored value means on", () => {
  assert.equal(parseAnimationPref(null), "on");
  assert.equal(parseAnimationPref("garbage"), "on");
  assert.equal(parseAnimationPref("system"), "system");
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
  assert.equal(migrateLegacyPref(null), "on");
  assert.equal(migrateLegacyPref("garbage"), "on");
});
