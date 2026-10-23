/**
 * Number formatting for counts shown in sentences. Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import { formatCount, truncatePath } from "./format.ts";

test("large counts are grouped", () => {
  assert.equal(formatCount(2530292, "en-US"), "2,530,292");
  assert.equal(formatCount(78934, "tr-TR"), "78.934");
});

test("small counts are unchanged", () => {
  assert.equal(formatCount(0, "en-US"), "0");
  assert.equal(formatCount(45, "en-US"), "45");
});

test("a short path is returned unchanged", () => {
  const path = String.raw`C:\Temp\a.tmp`;
  assert.equal(truncatePath(path, 48), path);
});

test("a long path with a short file name keeps the name whole", () => {
  const path = String.raw`C:\Users\QA\AppData\Local\Temp\winknife-qa-sentinel.tmp`;
  const out = truncatePath(path, 48);
  assert.ok(out.endsWith("winknife-qa-sentinel.tmp"), out);
  assert.ok(out.length <= 48, out);
  assert.equal(out.split("…").length, 2);
});

test("a file name longer than max still respects max", () => {
  const path = String.raw`C:\dir` + "\\" + "x".repeat(100);
  const out = truncatePath(path, 48);
  assert.ok(out.length <= 48, `${out.length}`);
  assert.equal(out.split("…").length, 2);
});

test("a path with no backslash uses the middle cut", () => {
  const out = truncatePath("a".repeat(60) + "b".repeat(60), 48);
  assert.equal(out, `${"a".repeat(23)}…${"b".repeat(23)}`);
});
