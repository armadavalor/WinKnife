// Kill-mid-clean consistency check for WinKnife (docs/QA-PLAN.md, "Restart mid-op").
//
// Starts a clean on the dirty fixture profile, force-kills WinKnife while files
// are being moved, relaunches it the way the CI job does, and verifies that the
// disk and the quarantine ledger agree: every file that was in Temp is either
// still at its original path or listed in the quarantine ledger with its
// quarantined copy present. A file that is in neither place is data loss from
// the user's point of view.
//
// THIS REALLY CLEANS AND KILLS THE APP. Run it only on a throwaway machine (the
// CI e2e job), never against a real user profile. Run it AFTER e2e.mjs, in the
// same shell step, so the profile env vars and the DevTools policy are shared.
//
// Usage: node scripts/qa/e2e-kill.mjs <fixture-manifest.json> [out-dir]
// Needs: APP (path to winknife.exe) and TEMP in the environment.
// Exit code: 1 if the disk and the ledger disagree (unless KNOWN_ISSUE is set).

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const PORT = 9222;
const STARTUP_TIMEOUT_MS = 120_000;
const STEP_TIMEOUT_MS = 60_000;
const SCAN_TIMEOUT_MS = 15 * 60_000;
const POLL_MS = 100;
const SAMPLE_SIZE = 300;
const KILL_WAIT_MS = 120_000;
// Set to a short description if a run shows a real bug, so CI reports it
// without failing (same convention as knownIssue in e2e.mjs).
const KNOWN_ISSUE = null;

const manifestPath = process.argv[2] ?? process.env.WINKNIFE_QA_MANIFEST;
const outDir = process.argv[3] ?? "e2e-out";
const app = process.env.APP;
if (!manifestPath || !app) {
  console.error("usage: APP=<winknife.exe> node scripts/qa/e2e-kill.mjs <fixture-manifest.json> [out-dir]");
  process.exit(2);
}
const manifest = JSON.parse(readFileSync(manifestPath, "utf8").replace(/^﻿/, ""));
mkdirSync(outDir, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const q = (v) => JSON.stringify(v ?? null);
const longOf = (p) => (p.startsWith("\\\\?\\") ? p : "\\\\?\\" + p);
const exists = (p) => existsSync(longOf(p));
const norm = (p) => p.replace(/^\\\\\?\\/, "").toLowerCase();

/* CDP plumbing, same approach as e2e.mjs. */

async function pageTarget(timeout = STARTUP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = list.find((t) => t.type === "page" && !t.url.startsWith("about:"));
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // not up yet
    }
    await sleep(1000);
  }
  throw new Error(`no WebView2 page target on port ${PORT}`);
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

let cdp = null;

async function attach() {
  cdp = await connect(await pageTarget());
  await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
  await waitFor('a[href="/settings"]', null, STARTUP_TIMEOUT_MS);
}

async function evaluate(expression) {
  const res = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (res.exceptionDetails) throw new Error(res.exceptionDetails.text + ": " + expression);
  return res.result.value;
}

const FIND = (sel, text) => `(() => {
  const vis = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  return [...document.querySelectorAll(${q(sel)})].find((el) => vis(el) && (!${q(text)} || el.textContent.includes(${q(text)}))) ?? null;
})()`;

const has = (sel, text) => evaluate(`!!${FIND(sel, text)}`);

async function waitFor(sel, text, timeout = STEP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await has(sel, text)) return;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${sel} "${text ?? ""}"`);
}

async function click(sel, text) {
  await waitFor(sel, text);
  const pos = await evaluate(`(() => {
    const el = ${FIND(sel, text)};
    el.scrollIntoView({ block: "nearest" });
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
  })()`);
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: pos.x, y: pos.y });
  for (const type of ["mousePressed", "mouseReleased"]) {
    await cdp.send("Input.dispatchMouseEvent", { type, x: pos.x, y: pos.y, button: "left", clickCount: 1 });
  }
}

/* Disk snapshot. */

function snapshotFiles(root) {
  const files = [];
  const walk = (dir) => {
    let entries;
    try {
      entries = readdirSync(longOf(dir), { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const full = join(dir, e.name);
      if (e.isFile()) files.push(full);
      else if (e.isDirectory() && !e.isSymbolicLink()) walk(full);
    }
  };
  walk(root);
  return files;
}

function relaunch() {
  const child = spawn(app, [], { detached: true, stdio: "ignore" });
  child.unref();
}

async function portClosed(timeout = 30_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      await fetch(`http://127.0.0.1:${PORT}/json/version`);
    } catch {
      return true;
    }
    await sleep(500);
  }
  return false;
}

const ledgerItems = () =>
  evaluate(`(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const items = [];
    for (const b of await invoke("quarantine_list")) {
      items.push(...(await invoke("quarantine_batch_items", { batchId: b.batchId })));
    }
    return items;
  })()`);

async function main() {
  const tempRoot = process.env.TEMP;
  const files = snapshotFiles(tempRoot);
  console.log(`snapshot: ${files.length} files under ${tempRoot}`);
  const step = Math.max(1, Math.floor(files.length / SAMPLE_SIZE));
  const sample = files.filter((_, i) => i % step === 0);

  await attach();
  // The sidebar groups tools in accordions; open the cleaner's tab first.
  if (!(await has('a[href="/cleaner"]'))) await click('nav button[data-tools~="cleaner"]');
  await click('a[href="/cleaner"]');
  await sleep(800);
  if (await has("main button", "Scan again")) await click("main button", "Scan again");
  await click("button", "Scan for junk");
  await waitFor("main button", "Clean up", SCAN_TIMEOUT_MS);
  await click("main button", "Clean up");
  await click('[role="alertdialog"] button', "Clean up");

  // Kill as soon as the first sampled file has left its place, so the move is
  // under way but (with thousands of files) nowhere near done.
  const deadline = Date.now() + KILL_WAIT_MS;
  let started = false;
  while (Date.now() < deadline && !started) {
    started = sample.some((p) => !exists(p));
    if (!started) await sleep(POLL_MS);
  }
  execFileSync("taskkill", ["/F", "/IM", "winknife.exe", "/T"], { stdio: "ignore" });
  cdp.close();
  const movedAtKill = files.filter((p) => !exists(p)).length;
  console.log(`killed WinKnife: move ${started ? "had started" : "had NOT started"}, ${movedAtKill}/${files.length} originals gone`);

  if (!(await portClosed())) throw new Error("DevTools port still open after taskkill");
  relaunch();
  await attach();
  await sleep(3000);

  const items = await ledgerItems();
  const byOriginal = new Map(items.map((i) => [norm(i.originalPath), i]));
  const problems = [];
  let atOriginal = 0;
  let inLedger = 0;
  for (const p of files) {
    if (exists(p)) {
      atOriginal++;
      continue;
    }
    const item = byOriginal.get(norm(p));
    if (item && exists(item.quarantinePath)) inLedger++;
    else problems.push({ path: p, reason: item ? "ledger row but quarantined file missing" : "gone and not in the ledger" });
  }
  // Ledger rows that point at a missing file and whose original is also absent.
  const dangling = items.filter((i) => !exists(i.quarantinePath) && !exists(i.originalPath));

  const result = {
    snapshot: files.length,
    movedAtKill,
    midway: started && movedAtKill > 0 && movedAtKill < files.length,
    atOriginal,
    inLedger,
    ledgerRows: items.length,
    lost: problems.length,
    lostSample: problems.slice(0, 20),
    danglingLedgerRows: dangling.length,
    knownIssue: KNOWN_ISSUE,
  };
  writeFileSync(join(outDir, "kill-results.json"), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));

  const bad = problems.length > 0 || dangling.length > 0;
  if (!result.midway) console.log("NOTE: the kill did not land mid-way; the check is weaker for this run.");
  if (bad && KNOWN_ISSUE) console.log(`xfail kill mid-clean: ${KNOWN_ISSUE}`);
  else if (bad) {
    console.log("fail  kill mid-clean: disk and ledger disagree");
    process.exitCode = 1;
  } else console.log("pass  kill mid-clean: disk and ledger agree");
  cdp.close();
}

try {
  await main();
} catch (err) {
  console.error(err);
  process.exitCode = 1;
}
