import assert from "node:assert/strict";
import test from "node:test";

import { resolveLocale } from "./localePref.ts";

test("a stored choice wins over the system language", () => {
  assert.equal(resolveLocale("en", "tr-TR"), "en");
  assert.equal(resolveLocale("tr", "en-US"), "tr");
});

test("without a stored choice, Turkish Windows gets Turkish", () => {
  assert.equal(resolveLocale(null, "tr-TR"), "tr");
  assert.equal(resolveLocale(null, "TR"), "tr");
});

test("anything else falls back to English", () => {
  assert.equal(resolveLocale(null, "en-US"), "en");
  assert.equal(resolveLocale("de", "de-DE"), "en");
  assert.equal(resolveLocale(null, undefined), "en");
});
