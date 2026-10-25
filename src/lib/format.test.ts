/**
 * Number formatting for counts shown in sentences. Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import { formatBytes, formatCount, formatRelativeDays, formatRemaining, truncatePath } from "./format.ts";

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

test("byte sizes follow the locale's decimal mark, units stay", () => {
  assert.equal(formatBytes(1536, "en-US"), "1.5 KB");
  assert.equal(formatBytes(1536, "tr-TR"), "1,5 KB");
  assert.equal(formatBytes(0, "tr-TR"), "0 B");
  assert.equal(formatBytes(50 * 1024 * 1024, "tr-TR"), "50 MB");
});

test("durations and day counts are localized", () => {
  assert.equal(formatRemaining(3725, "en-US"), "1h 02m 05s");
  assert.equal(formatRemaining(3725, "tr-TR"), "1sa 02dk 05sn");
  const from = new Date("2026-01-01T00:00:00Z");
  assert.equal(formatRelativeDays("2026-01-04T00:00:00Z", from, "tr-TR"), "3 gün");
  assert.equal(formatRelativeDays("2026-01-04T00:00:00Z", from, "en-US"), "3 days");
});
