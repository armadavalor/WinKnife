/**
 * Reason keys resolve through the dictionary and fall back to the English
 * `reason`. Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import { resolveReason } from "./reason.ts";

const dict: Record<string, string> = {
  "brain.protected": "korumalı: {why}",
  "protected.windows": "Windows klasörü",
  "brain.downgraded.lowScore": "{rule} {score} puan",
  "rule.p.r": "kural",
};
const lookup = (key: string): string | undefined => dict[key];

test("no key shows the English reason", () => {
  assert.equal(resolveReason({ reason: "english" }, lookup), "english");
});

test("a known key is translated with nested and number params", () => {
  assert.equal(
    resolveReason(
      { reason: "x", reasonKey: "brain.protected", reasonParams: { why: { key: "protected.windows" } } },
      lookup,
    ),
    "korumalı: Windows klasörü",
  );
  assert.equal(
    resolveReason(
      { reason: "x", reasonKey: "brain.downgraded.lowScore", reasonParams: { rule: { key: "rule.p.r" }, score: 40 } },
      lookup,
    ),
    "kural 40 puan",
  );
});

test("an unknown key falls back to the English reason", () => {
  assert.equal(resolveReason({ reason: "english", reasonKey: "nope" }, lookup), "english");
});

test("an unknown nested key falls back to the English reason", () => {
  assert.equal(
    resolveReason(
      { reason: "english", reasonKey: "brain.protected", reasonParams: { why: { key: "protected.nope" } } },
      lookup,
    ),
    "english",
  );
});
