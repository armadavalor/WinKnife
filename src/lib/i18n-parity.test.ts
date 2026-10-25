/**
 * English and Turkish must stay in lockstep: same keys, same `{placeholder}`
 * tokens. Source-level check because i18n.ts needs the `$lib` alias.
 * Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

const source = readFileSync(join(import.meta.dirname, "i18n.ts"), "utf8");
const enStart = source.indexOf("const en = {");
const trStart = source.indexOf("const tr:");
assert.ok(enStart >= 0 && trStart > enStart, "could not locate the en/tr blocks in i18n.ts");

// A value is a string literal (possibly on the next line) or a bare constant such as APP_NAME.
const ENTRY = /^\s*"([\w.-]+)":\s*("(?:[^"\\]|\\.)*"|[A-Z_]+)/gm;

function entries(block: string): Map<string, string[]> {
  const map = new Map<string, string[]>();
  for (const m of block.matchAll(ENTRY)) {
    map.set(m[1], [...m[2].matchAll(/\{(\w+)\}/g)].map((p) => p[1]).sort());
  }
  return map;
}

const en = entries(source.slice(enStart, trStart));
const tr = entries(source.slice(trStart));

test("both dictionaries were scanned", () => {
  assert.ok(en.size > 500, `only ${en.size} English keys found`);
  assert.ok(tr.size > 500, `only ${tr.size} Turkish keys found`);
});

test("every English key has a Turkish value", () => {
  const missing = [...en.keys()].filter((k) => !tr.has(k));
  assert.deepEqual(missing, [], `missing in tr: ${missing.join(", ")}`);
});

test("every Turkish key exists in English", () => {
  const extra = [...tr.keys()].filter((k) => !en.has(k));
  assert.deepEqual(extra, [], `only in tr: ${extra.join(", ")}`);
});

test("placeholder tokens match between languages", () => {
  const mismatched = [...en]
    .filter(([key, tokens]) => tr.has(key) && tr.get(key)!.join() !== tokens.join())
    .map(([key, tokens]) => `${key}: en {${tokens}} vs tr {${tr.get(key)}}`);
  assert.deepEqual(mismatched, []);
});
