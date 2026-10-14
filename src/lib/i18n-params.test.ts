/**
 * Every `t("key")` call whose string has `{placeholders}` must pass params,
 * otherwise the raw "{count}" reaches the screen (cleaner confirm dialog,
 * v0.2.0). Source-level check because i18n.ts needs the `$lib` alias.
 * Run with `npm run test:ui`.
 */
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

const SRC = join(import.meta.dirname, "..");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    const isSource = /\.(svelte|ts)$/.test(entry.name);
    const isExcluded = entry.name === "i18n.ts" || entry.name.includes(".test.");
    return isSource && !isExcluded ? [path] : [];
  });
}

function placeholderKeys(): Map<string, string[]> {
  const dictionary = readFileSync(join(SRC, "lib", "i18n.ts"), "utf8");
  const keys = new Map<string, string[]>();
  for (const m of dictionary.matchAll(/^\s*"([\w.]+)":\s*"((?:[^"\\]|\\.)*)"/gm)) {
    const names = [...m[2].matchAll(/\{(\w+)\}/g)].map((p) => p[1]);
    if (names.length > 0) keys.set(m[1], names);
  }
  return keys;
}

test("t() calls for keys with placeholders pass params", () => {
  const keys = placeholderKeys();
  assert.ok(keys.size > 20, "dictionary scan found no placeholder keys");
  const files = sourceFiles(SRC).map((path) => [path, readFileSync(path, "utf8")]);
  const bare: string[] = [];
  for (const [key] of keys) {
    const call = new RegExp(`\\bt\\(\\s*["'\`]${key.replace(/\./g, "\\.")}["'\`]\\s*\\)`);
    for (const [path, text] of files) {
      if (call.test(text)) bare.push(`${key} in ${path}`);
    }
  }
  assert.deepEqual(bare, []);
});
