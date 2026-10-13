/**
 * Number formatting for counts shown in sentences. Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import { formatCount } from "./format.ts";

test("large counts are grouped", () => {
  assert.equal(formatCount(2530292, "en-US"), "2,530,292");
  assert.equal(formatCount(78934, "tr-TR"), "78.934");
});

test("small counts are unchanged", () => {
  assert.equal(formatCount(0, "en-US"), "0");
  assert.equal(formatCount(45, "en-US"), "45");
});
