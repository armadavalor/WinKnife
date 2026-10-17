// End-to-end checks for WinKnife on a dirty fixture profile (docs/QA-PLAN.md).
//
// Drives an installed WinKnife over the WebView2 DevTools protocol, like
// scripts/demo/record.mjs. The app must already be running with
// --remote-debugging-port=9222 and with its profile env vars pointed at a
// profile made by scripts/qa/New-DirtyProfile.ps1.
//
// THIS REALLY CLEANS: the cleaner test moves the fixture's junk to quarantine.
// Run it only on a throwaway machine (the CI e2e job, Windows Sandbox), never
// against a real user profile.
//
// Usage: node scripts/qa/e2e.mjs <fixture-manifest.json> [out-dir]
// Output: <out-dir>/results.json plus a screenshot per failed test.
// Exit code: 1 if any test not marked knownIssue failed.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const PORT = 9222;
const WIDTH = 1280;
const HEIGHT = 800;
const STARTUP_TIMEOUT_MS = 120_000;
const STEP_TIMEOUT_MS = 60_000;
const SCAN_TIMEOUT_MS = 15 * 60_000;
const POLL_MS = 250;
const SETTLE_MS = 800;
const PROTECTED_FILE = "C:\\Windows\\notepad.exe";

const manifestPath = process.argv[2] ?? process.env.WINKNIFE_QA_MANIFEST;
if (!manifestPath) {
  console.error("usage: node scripts/qa/e2e.mjs <fixture-manifest.json> [out-dir]");
  process.exit(2);
}
const manifest = JSON.parse(readFileSync(manifestPath, "utf8").replace(/^\uFEFF/, ""));
const fx = manifest.sentinels;
const outDir = process.argv[3] ?? "e2e-out";
mkdirSync(outDir, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const q = (v) => JSON.stringify(v ?? null);

/* ---------------------------------------------------------------------------
 * CDP plumbing (same approach as scripts/demo/record.mjs)
 * ------------------------------------------------------------------------- */

async function pageTarget() {
  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  let last = "no response";
  while (Date.now() < deadline) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = list.find((t) => t.type === "page" && !t.url.startsWith("about:"));
      if (page) return page.webSocketDebuggerUrl;
      last = JSON.stringify(list.map((t) => [t.type, t.url]));
    } catch (err) {
      last = String(err.cause?.code ?? err.message);
    }
    await sleep(1000);
  }
  throw new Error(`no WebView2 page target on port ${PORT} (last: ${last})`);
}

function connect(url) {
  const ws = new WebSocket(url);
  const pending = new Map();
  let nextId = 1;
  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data);
    const waiter = pending.get(msg.id);
    if (!waiter) return;
    pending.delete(msg.id);
    if (msg.error) waiter.reject(new Error(JSON.stringify(msg.error)));
    else waiter.resolve(msg.result);
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ id, method, params }));
    });
  return new Promise((resolve, reject) => {
    ws.onopen = () => resolve({ send, close: () => ws.close() });
    ws.onerror = () => reject(new Error("CDP websocket failed"));
  });
}

const cdp = await connect(await pageTarget());

async function evaluate(expression) {
  const res = await cdp.send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (res.exceptionDetails) throw new Error(res.exceptionDetails.text + ": " + expression);
  return res.result.value;
}

// Visible element lookup by selector + text. Re-installed after every
// navigation check, since a full reload would drop it.
const HELPERS = `
window.__qa = {
  visible(el) {
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  },
  find(sel, text, within) {
    const root = within ? document.querySelector(within) : document;
    if (!root) return null;
    return [...root.querySelectorAll(sel)].find(
      (el) => this.visible(el) && (!text || el.textContent.includes(text)),
    ) ?? null;
  },
  center(sel, text, within) {
    const el = this.find(sel, text, within);
    if (!el) return null;
    el.scrollIntoView({ block: "nearest" });
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
  },
  setInput(sel, value) {
    const el = document.querySelector(sel);
    if (!el) return false;
    el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  },
};
true;
`;

const ensureHelpers = () => evaluate(`if (!window.__qa) { ${HELPERS} } true;`);

async function has(sel, text, within) {
  await ensureHelpers();
  return evaluate(`!!window.__qa.find(${q(sel)}, ${q(text)}, ${q(within)})`);
}

async function waitFor(sel, text, within, timeout = STEP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await has(sel, text, within)) return;
    await sleep(POLL_MS);
  }
  throw new Error(`timed out after ${timeout} ms waiting for ${sel} "${text ?? ""}"`);
}

// Resolves with the index of the first of `options` ([sel, text, within]) to appear.
async function waitForAny(options, timeout = STEP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const [i, [sel, text, within]] of options.entries()) {
      if (await has(sel, text, within)) return i;
    }
    await sleep(POLL_MS);
  }
  throw new Error(`timed out after ${timeout} ms waiting for any of ${JSON.stringify(options)}`);
}

async function waitGone(sel, text, within, timeout = STEP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (!(await has(sel, text, within))) return;
    await sleep(POLL_MS);
  }
  throw new Error(`timed out after ${timeout} ms waiting for ${sel} "${text ?? ""}" to go away`);
}

async function click(sel, text, within) {
  await waitFor(sel, text, within);
  const pos = await evaluate(`window.__qa.center(${q(sel)}, ${q(text)}, ${q(within)})`);
  if (!pos) throw new Error(`no element ${sel} "${text ?? ""}"`);
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: pos.x, y: pos.y });
  for (const type of ["mousePressed", "mouseReleased"]) {
    await cdp.send("Input.dispatchMouseEvent", { type, x: pos.x, y: pos.y, button: "left", clickCount: 1 });
  }
}

async function goTo(route) {
  await click(`a[href="/${route}"]`);
  await sleep(SETTLE_MS);
}

async function setInput(sel, value) {
  await waitFor(sel);
  if (!(await evaluate(`window.__qa.setInput(${q(sel)}, ${q(value)})`))) {
    throw new Error(`no input ${sel}`);
  }
}

async function screenshot(name) {
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(outDir, `${name}.png`), Buffer.from(data, "base64"));
}

function expect(condition, message) {
  if (!condition) throw new Error(message);
}

const exists = (p) => p && existsSync(p);

/* ---------------------------------------------------------------------------
 * Tests. Order matters: quarantine restore uses the cleaner's batch.
 * knownIssue: a filed bug; a failure is reported but does not fail the run.
 * ------------------------------------------------------------------------- */

const tests = [];
const test = (name, fn, opts = {}) => tests.push({ name, fn, ...opts });

test("cleaner: scan, clean safe items, nothing protected or fresh touched", async () => {
  expect(exists(fx.tempOld), `fixture missing ${fx.tempOld}`);
  await goTo("cleaner");
  await click("button", "Scan for junk");
  await waitFor("main button", "Clean up", null, SCAN_TIMEOUT_MS);
  await click("main button", "Clean up");
  await click('[role="alertdialog"] button', "Clean up");
  await waitFor("button", "Open quarantine", null, SCAN_TIMEOUT_MS);
  expect(!exists(fx.tempOld), `old temp sentinel still in place: ${fx.tempOld}`);
  expect(exists(fx.tempFresh), `fresh temp file was moved: ${fx.tempFresh}`);
  expect(exists(fx.downloadsRecent), `recent download was moved: ${fx.downloadsRecent}`);
  expect(exists(PROTECTED_FILE), `${PROTECTED_FILE} is gone`);
});

test("quarantine: restore the cleaner batch", async () => {
  await click("button", "Open quarantine");
  await click("button", "Restore", 'aside[aria-label="Quarantine"]');
  await waitFor("*", "Restored ", 'aside[aria-label="Quarantine"]', SCAN_TIMEOUT_MS);
  expect(exists(fx.tempOld), `old temp sentinel not restored: ${fx.tempOld}`);
  await click("button", "Close", 'aside[aria-label="Quarantine"]');
  await sleep(SETTLE_MS);
});

test("empty folders: finds the fixture tree, keeps the folder with a file", async () => {
  await goTo("empty-folders");
  await setInput('main input[type="text"]', fx.emptyRoot);
  await click("button", "Find empty folders");
  const which = await waitForAny(
    [["*", "empty folders found"], ["*", "No empty folders found."]],
    SCAN_TIMEOUT_MS,
  );
  expect(which === 0, "scan of the fixture tree found no empty folders");
  expect(exists(fx.emptyKeep), `scan touched ${fx.emptyKeep}`);
});

// A drive root is refused as a scan root, and the protected trees are skipped,
// so Stop needs a big unprotected folder: the runner's tool cache, or
// WINKNIFE_QA_BIG_DIR (e.g. a large folder inside Windows Sandbox).
const BIG_DIR = [process.env.WINKNIFE_QA_BIG_DIR, "C:\\hostedtoolcache"].find((d) => d && exists(d));

test("empty folders: Stop on a big folder", async () => {
  expect(BIG_DIR, "no big folder to scan; set WINKNIFE_QA_BIG_DIR");
  await goTo("empty-folders");
  if (await has("button", "Scan again")) await click("button", "Scan again");
  await setInput('main input[type="text"]', BIG_DIR);
  await click("button", "Find empty folders");
  await click("button", "Stop");
  await waitFor("*", "The scan was stopped early", null, STEP_TIMEOUT_MS);
});

test("duplicates: finds the fixture's copy sets", async () => {
  await goTo("duplicates");
  await click("button", "Find duplicates");
  const which = await waitForAny(
    [["*", "sets of copies"], ["*", "No duplicates in the folders"]],
    SCAN_TIMEOUT_MS,
  );
  expect(which === 0, "no duplicate sets found in the fixture");
});

test(
  "duplicates: Stop returns to idle",
  async () => {
    await goTo("duplicates");
    await click("button", "Scan again");
    await click("button", "Stop");
    await waitGone("button", "Stopping", null, STEP_TIMEOUT_MS);
    await waitForAny([["main button", "Find duplicates"], ["main button", "Scan again"]], STEP_TIMEOUT_MS);
  },
  { knownIssue: "Stop uses a stale scan id (QA-PLAN audit A2)" },
);

test("treemap: map C:, Stop, button comes back", async () => {
  await goTo("treemap");
  await click("button", "Map ");
  await click("button", "Stop");
  await waitFor("button", "Map ", null, STEP_TIMEOUT_MS);
});

test("keep awake: start and stop", async () => {
  await goTo("keep-awake");
  await click("main button", "30 minutes");
  await waitFor("*", "Staying awake for");
  await click("main button", "Stop");
  await waitFor("*", "The PC can sleep as usual.");
});

test("file unlocker: names the process holding the locked file", async () => {
  expect(manifest.lockHelperPid, "fixture has no lock helper");
  await goTo("file-unlocker");
  await setInput("#unlocker-path", fx.locked);
  await click("button", "Add");
  const which = await waitForAny(
    [
      ["*", "of these files open"],
      ["*", "Holds one or more of these files"],
      ["*", "No program is holding these files open."],
      ["*", "Windows could not answer the question."],
    ],
    STEP_TIMEOUT_MS,
  );
  expect(which <= 1, "the locked fixture file was not reported as held");
});

for (const [route, loading, error] of [
  ["startup", "Reading what starts with Windows", "Could not read what starts with Windows."],
  ["legacy-panels", "Finding the panels on this PC", null],
  ["restore-point", "Checking System Protection", null],
]) {
  test(`${route}: page loads`, async () => {
    await goTo(route);
    await waitGone("*", loading, "main", STEP_TIMEOUT_MS);
    if (error) expect(!(await has("*", error, "main")), `${route} shows "${error}"`);
  });
}

/* ---------------------------------------------------------------------------
 * Runner
 * ------------------------------------------------------------------------- */

async function main() {
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: WIDTH,
    height: HEIGHT,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await waitFor('a[href="/cleaner"]', null, null, STARTUP_TIMEOUT_MS);

  const only = process.env.WINKNIFE_QA_ONLY;
  const results = [];
  for (const [i, t] of tests.entries()) {
    if (only && !t.name.startsWith(only)) continue;
    const started = Date.now();
    let error = null;
    try {
      await t.fn();
    } catch (err) {
      error = err.message;
      await screenshot(`fail-${String(i).padStart(2, "0")}`).catch(() => {});
    }
    const ms = Date.now() - started;
    const status = error ? (t.knownIssue ? "xfail" : "fail") : t.knownIssue ? "xpass" : "pass";
    results.push({ name: t.name, status, ms, error, knownIssue: t.knownIssue ?? null });
    console.log(`${status.padEnd(5)} ${t.name} (${(ms / 1000).toFixed(1)} s)${error ? `\n      ${error}` : ""}`);
  }

  writeFileSync(join(outDir, "results.json"), JSON.stringify({ manifest: manifest.counts, results }, null, 2));
  const failed = results.filter((r) => r.status === "fail").length;
  console.log(`\n${results.length - failed}/${results.length} ok (${failed} failed)`);
  if (failed) process.exitCode = 1;
}

try {
  await main();
} catch (err) {
  await screenshot("fatal").catch(() => {});
  console.error(err);
  process.exitCode = 1;
} finally {
  cdp.close();
}
