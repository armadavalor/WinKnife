/**
 * Background light variants (Polish item 7). Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import { GLOW_VARIANTS, glowPointer, parseGlowVariant } from "./glow.ts";

test("parseGlowVariant keeps known variants and defaults the rest to aurora", () => {
  for (const variant of GLOW_VARIANTS) assert.equal(parseGlowVariant(variant), variant);
  assert.equal(parseGlowVariant(null), "aurora");
  assert.equal(parseGlowVariant("disco"), "aurora");
  assert.equal(parseGlowVariant(3), "aurora");
});

test("there are at least three variants to pick from", () => {
  assert.ok(GLOW_VARIANTS.length >= 3);
});

test("glowPointer turns a pointer position into percentages of the window", () => {
  assert.deepEqual(glowPointer(0, 0, 1000, 500), { x: 0, y: 0 });
  assert.deepEqual(glowPointer(500, 250, 1000, 500), { x: 50, y: 50 });
  assert.deepEqual(glowPointer(1000, 500, 1000, 500), { x: 100, y: 100 });
});

test("glowPointer clamps outside positions and survives a zero-size window", () => {
  assert.deepEqual(glowPointer(-40, 900, 1000, 500), { x: 0, y: 100 });
  assert.deepEqual(glowPointer(10, 10, 0, 0), { x: 50, y: 50 });
});
