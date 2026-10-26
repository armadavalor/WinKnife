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

import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { spawn } from "node:child_process";
import { basename, dirname, join } from "node:path";

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
  const tab = `nav [role="tab"][data-tools~="${route}"]`;
  // The sidebar lists one category tab at a time; open the tool's tab first.
  if (!(await has(`a[href="/${route}"]`)) && (await has(tab))) await click(tab);
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

// Node adds the \\?\ prefix itself for long absolute paths; spell it out so a
// path over 260 characters resolves whatever the OS long-path policy says.
const longOf = (p) => (p.startsWith("\\\\?\\") ? p : "\\\\?\\" + p);
const existsLong = (p) => existsSync(longOf(p));

const DRAWER = 'aside[aria-label="Quarantine"]';
const drawerOpen = () => evaluate(`!!document.querySelector(${q(DRAWER)})`);

async function openDrawer() {
  if (!(await drawerOpen())) await click("button", "Quarantine");
  await waitFor("*", null, DRAWER);
  await waitGone("*", "Loading", DRAWER).catch(() => {});
  await sleep(SETTLE_MS);
}

async function closeDrawer() {
  if (await drawerOpen()) {
    await click("button", "Close", DRAWER);
    await sleep(SETTLE_MS);
  }
}

// Restores the first batch the drawer lists (the newest) and waits for the
// report. Returns the drawer's text once the report is on screen.
async function restoreFirstBatch() {
  await openDrawer();
  await click("button", "Restore", DRAWER);
  await waitFor("*", "Restored ", DRAWER, SCAN_TIMEOUT_MS);
  await sleep(SETTLE_MS);
  return evaluate(`document.querySelector(${q(DRAWER)}).innerText`);
}

// Every item the quarantine ledger still holds, read from the backend.
// quarantine_batch_items also returns the rows of a part-restored batch that are already back
// on disk (restoredAt set), so those are filtered out.
const ledgerItems = () =>
  evaluate(`(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const items = [];
    for (const b of await invoke("quarantine_list")) {
      items.push(...(await invoke("quarantine_batch_items", { batchId: b.batchId })));
    }
    return items.filter((i) => !i.restoredAt);
  })()`);

// Scans the cleaner and cleans what it ticked; the batch stays in quarantine.
async function cleanTemp() {
  await goTo("cleaner");
  if (await has("main button", "Scan again")) await click("main button", "Scan again");
  await click("button", "Scan for junk");
  await waitFor("main button", "Clean up", null, SCAN_TIMEOUT_MS);
  await click("main button", "Clean up");
  await click('[role="alertdialog"] button', "Clean up");
  await waitFor("button", "Open quarantine", null, SCAN_TIMEOUT_MS);
}

// Puts every held batch back through the backend, so one test's failure cannot
// leave items in quarantine for the next.
const restoreAllHeld = () =>
  evaluate(`(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    for (const b of await invoke("quarantine_list")) {
      await invoke("quarantine_restore", { batchId: b.batchId });
    }
  })()`);

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

test("empty folders: quarantine the found folders, then restore them", async () => {
  const buildDir = join(fx.emptyRoot, "proj000", "build");
  const deepest = join(buildDir, "out0", "empty");
  expect(exists(deepest), `fixture missing ${deepest}`);
  await goTo("empty-folders");
  if (await has("main button", "Scan again")) await click("main button", "Scan again");
  await setInput('main input[type="text"]', fx.emptyRoot);
  await click("button", "Find empty folders");
  await waitFor("*", "empty folders found", null, SCAN_TIMEOUT_MS);
  await click("main button", "Select all");
  await click("main button", "Move folders to quarantine");
  await click('[role="alertdialog"] button', "Move folders to quarantine");
  await waitFor("*", "moved to quarantine", null, SCAN_TIMEOUT_MS);
  expect(!exists(buildDir), `empty folder tree still in place: ${buildDir}`);
  // The folder that holds a file must survive, file included.
  expect(exists(fx.emptyKeep), `folder with a file was removed: ${fx.emptyKeep}`);
  expect(exists(join(fx.emptyKeep, "main.rs")), "main.rs vanished from the kept folder");
  await restoreFirstBatch();
  expect(exists(deepest), `empty folder not restored: ${deepest}`);
  await closeDrawer();
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
  // Stop before the first progress event is a no-op (audit A9); wait for the
  // walker's current path so this tests Stop itself.
  await waitFor("main *", BIG_DIR + "\\");
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

test("duplicates: quarantine one copy, twins stay, restore brings it back", async () => {
  // Sets render collapsed; open them all so the copies' checkboxes exist.
  await evaluate(`(() => {
    for (const b of document.querySelectorAll('main button[aria-expanded="false"]')) b.click();
    return true;
  })()`);
  await sleep(SETTLE_MS);
  // The scan shows only some of the fixture's sets, so take the first set
  // that offers a non-keeper copy; its other copies are the twins.
  const pick = await evaluate(`(() => {
    const boxes = document.querySelectorAll('main section input[type="checkbox"]');
    const diag = { boxes: boxes.length, collapsed: document.querySelectorAll('main button[aria-expanded="false"]').length };
    const box = boxes[0];
    if (!box) return { diag };
    const paths = [...box.closest("section").querySelectorAll("span[title]")].map((e) => e.getAttribute("title")).filter((x) => /^[A-Za-z]:\\\\/.test(x));
    const target = box.getAttribute("aria-label");
    if (!box.checked) box.click();
    return { target, twins: paths.filter((x) => x !== target), diag };
  })()`);
  expect(pick.target, `no non-keeper checkbox on the page: ${JSON.stringify(pick.diag)}`);
  const target = pick.target;
  const twins = pick.twins;
  expect(twins.length >= 1 && twins.every((p) => exists(p)), `twins missing before the move: ${JSON.stringify(pick)}`);
  await sleep(SETTLE_MS);
  await click("main button", "Move copies to quarantine");
  await click('[role="alertdialog"] button', "Move copies to quarantine");
  await waitFor("button", "Open quarantine", null, SCAN_TIMEOUT_MS);
  expect(!exists(target), `quarantined copy still on disk: ${target}`);
  for (const twin of twins) expect(exists(twin), `a twin was removed too: ${twin}`);
  await restoreFirstBatch();
  expect(exists(target), `copy not restored: ${target}`);
  for (const twin of twins) expect(exists(twin), `a twin vanished on restore: ${twin}`);
  await closeDrawer();
});

test("duplicates: Stop cancels the scan", async () => {
  // The fixture scan ends before Stop renders, so scan a big folder through
  // the treemap's `?root=` cross-link.
  expect(BIG_DIR, "no big folder to scan; set WINKNIFE_QA_BIG_DIR");
  const href = `/duplicates?root=${encodeURIComponent(BIG_DIR)}`;
  await evaluate(
    `(() => { const a = document.createElement("a"); a.href = ${q(href)}; ` +
      `document.body.append(a); a.click(); a.remove(); return true; })()`,
  );
  await sleep(SETTLE_MS);
  // "Scan again" only resets the page to idle; "Find duplicates" starts the scan.
  if (await has("main button", "Scan again")) await click("main button", "Scan again");
  await waitFor("main button", "Find duplicates");
  await ensureHelpers();
  // Arm the observer before starting the scan. A CDP round trip plus 250 ms
  // polling can miss both the first progress path and the short-lived Stop UI.
  // Stop before the first progress event is supported: the store sends cancel
  // when that event supplies the scan id.
  const outcome = await evaluate(`new Promise((resolve) => {
    const main = document.querySelector("main");
    const start = window.__qa.find("main button", "Find duplicates");
    const observer = new MutationObserver(check);
    const timeout = setTimeout(() => finish("Stop never appeared"), ${STEP_TIMEOUT_MS});
    function finish(value) {
      observer.disconnect();
      clearTimeout(timeout);
      resolve(value);
    }
    function check() {
      const stop = window.__qa.find("main button", "Stop");
      if (stop) {
        stop.click();
        finish("stopped");
      } else if (window.__qa.find("main button", "Scan again")) {
        finish("scan completed before Stop appeared");
      }
    }
    observer.observe(main, { childList: true, subtree: true, characterData: true });
    start.click();
    check();
  })`);
  expect(outcome === "stopped", outcome);
  await waitFor("main button", "Scan again", null, STEP_TIMEOUT_MS);
  await waitFor("main *", "Stopped before it finished", null, STEP_TIMEOUT_MS);
});

test("cleaner: unicode and >260-char paths go to quarantine and come back by exact name", async () => {
  expect(existsLong(fx.unicode), `fixture missing ${fx.unicode}`);
  expect(existsLong(fx.longPath), `fixture missing the long-path file`);
  expect(fx.longPath.length > 260, `long-path fixture is only ${fx.longPath.length} characters`);
  await cleanTemp();
  expect(!existsLong(fx.unicode), `unicode file not quarantined: ${fx.unicode}`);
  expect(!existsLong(fx.longPath), "long-path file not quarantined");
  const report = await restoreFirstBatch();
  expect(existsLong(fx.unicode), `unicode file not restored: ${fx.unicode}`);
  expect(existsLong(fx.longPath), "long-path file not restored");
  // Exact names, not a lookalike: the directory listing must hold the stored name.
  for (const p of [fx.unicode, fx.longPath]) {
    const names = readdirSync(longOf(dirname(p)));
    expect(names.includes(basename(p)), `restored name differs from ${basename(p)}; found ${names}`);
  }
  expect(!/could not|failed/i.test(report), `restore report mentions a failure: ${report.slice(0, 300)}`);
  await closeDrawer();
});

test("quarantine: restore onto an occupied destination loses nothing", async () => {
  const occupant = Buffer.from("not-the-original");
  await cleanTemp();
  expect(!exists(fx.tempOld), "sentinel was not quarantined");
  writeFileSync(fx.tempOld, occupant);
  const report = await restoreFirstBatch();
  // The occupant is somebody else's file: it must be untouched.
  expect(
    exists(fx.tempOld) && readFileSync(fx.tempOld).equals(occupant),
    "the file occupying the destination was replaced or removed",
  );
  // The quarantined copy must survive somewhere: restored beside the occupant
  // under another name, or still in the ledger with its file present.
  const dir = dirname(fx.tempOld);
  const stem = basename(fx.tempOld, ".tmp");
  const beside = readdirSync(dir).filter(
    (n) => n.startsWith(stem) && join(dir, n) !== fx.tempOld && sizeOf(join(dir, n)) === 2048,
  );
  const held = (await ledgerItems()).filter((i) => i.originalPath.toLowerCase() === fx.tempOld.toLowerCase());
  const keptInLedger = held.some((i) => exists(i.quarantinePath));
  expect(
    beside.length > 0 || keptInLedger,
    `the quarantined sentinel is neither beside the occupant nor in the ledger. Drawer: ${report.slice(0, 300)}`,
  );
  // Put the sentinel back for later tests: drop the occupant, rename the sibling.
  rmSync(fx.tempOld);
  if (beside.length > 0) renameSync(join(dir, beside[0]), fx.tempOld);
  else if (keptInLedger) {
    await closeDrawer();
    await restoreFirstBatch();
  }
  expect(exists(fx.tempOld), "could not put the sentinel back");
  await closeDrawer();
});

const sizeOf = (p) => statSync(p).size;

// The two Busy tests share one run: the drawer's text can only be observed
// while the double-click is happening.
let busyRun = null;

test("quarantine: a second restore while one runs is refused (Busy), one restore applied", async () => {
  await cleanTemp();
  // Part 1, backend: two concurrent restores of one batch. The guard must
  // refuse exactly one and the other must restore everything.
  const direct = await evaluate(`(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const [batch] = await invoke("quarantine_list");
    const settle = (p) => p.then((v) => ({ state: "ok", restored: v.restored.length, failed: v.failed.length }),
      (e) => ({ state: "err", message: typeof e === "string" ? e : JSON.stringify(e) }));
    return Promise.all([settle(invoke("quarantine_restore", { batchId: batch.batchId })),
      settle(invoke("quarantine_restore", { batchId: batch.batchId }))]);
  })()`);
  const ok = direct.filter((c) => c.state === "ok");
  const refused = direct.filter((c) => c.state === "err");
  expect(ok.length === 1 && refused.length === 1, `expected one success and one refusal: ${JSON.stringify(direct)}`);
  expect(/already running/i.test(refused[0].message), `refusal is not the Busy message: ${refused[0].message}`);
  expect(ok[0].failed === 0 && ok[0].restored > 0, `the applied restore reported ${JSON.stringify(ok[0])}`);
  expect(exists(fx.tempOld), "sentinel not restored by the applied restore");
  expect((await ledgerItems()).length === 0, "items left in the ledger after the only restore");

  // Part 2, UI: a restore started behind the drawer's back holds the lock;
  // the drawer's own Restore click is refused. Watch whether the user can see it.
  await cleanTemp();
  await openDrawer();
  busyRun = await evaluate(`(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const [batch] = await invoke("quarantine_list");
    const aside = document.querySelector(${q(DRAWER)});
    let sawText = false;
    const watcher = setInterval(() => {
      if (aside.innerText.includes("already running")) sawText = true;
    }, 10);
    const hidden = invoke("quarantine_restore", { batchId: batch.batchId }).then(() => "ok", (e) => String(e));
    window.__qa.find("button", "Restore", ${q(DRAWER)}).click();
    const outcome = await hidden;
    await new Promise((r) => setTimeout(r, 1500));
    clearInterval(watcher);
    return { outcome, sawText };
  })()`);
  expect(busyRun.outcome === "ok", `the background restore did not apply: ${busyRun.outcome}`);
  expect((await ledgerItems()).length === 0, "items left in the ledger after the background restore");
  await closeDrawer();
});

test("quarantine: the drawer shows the Busy refusal to the user", async () => {
  expect(busyRun, "the double-restore test did not run");
  expect(busyRun.sawText, "no 'already running' text was ever visible in the drawer");
});

// Presses Stop once the run in `within` shows "N of M" with 0 < N < M. `start`
// is run first, in the page, so the observer cannot miss a short-lived Stop.
// `confirm` (optional) is clicked in the page as soon as it appears. Returns
// "stopped", or why it could not stop.
async function stopMidRun({ within, start, confirm }) {
  await ensureHelpers();
  return evaluate(`new Promise((resolve) => {
    const scope = () => document.querySelector(${q(within)});
    const observer = new MutationObserver(check);
    const timeout = setTimeout(() => finish("Stop never appeared mid-run"), ${STEP_TIMEOUT_MS});
    function finish(value) {
      observer.disconnect();
      clearTimeout(timeout);
      resolve(value);
    }
    function check() {
      const root = scope();
      if (!root) return;
      ${confirm ? `const ok = window.__qa.find(${q(confirm.sel)}, ${q(confirm.text)}); if (ok) ok.click();` : ""}
      // Only the progress line itself: other text in the scope can contain "N of M" too.
      const m = [...root.querySelectorAll("p")].map((p) => /^(\\d+) of (\\d+)$/.exec(p.innerText.trim())).find(Boolean);
      const stop = window.__qa.find("button", "Stop", ${q(within)});
      if (m && stop && Number(m[1]) > 0 && Number(m[1]) < Number(m[2])) {
        stop.click();
        finish("stopped");
      } else if (m && Number(m[1]) >= Number(m[2])) {
        finish("run finished before Stop could be pressed");
      }
    }
    observer.observe(document.body, { childList: true, subtree: true, characterData: true });
    ${start};
    check();
  })`);
}

// Mirrors src/lib/format.ts, which the page uses for the done view.
function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : value < 10 ? 1 : 0)} ${units[unit]}`;
}

const CLEAN_BUTTON =`window.__qa.find("main button", "Clean up").click()`;
const mainText = () => evaluate(`document.querySelector("main").innerText`);

const scanCleaner = async () => {
  await goTo("cleaner");
  if (await has("main button", "Scan again")) await click("main button", "Scan again");
  await click("button", "Scan for junk");
  await waitFor("main button", "Clean up", null, SCAN_TIMEOUT_MS);
};

// Holds `path` open with FileShare.None from a separate process (Node's fs.open shares, so it cannot lock).
const holdExclusive = (path) =>
  new Promise((resolve, reject) => {
    const script = `$f=[IO.File]::Open('${path.replaceAll("'", "''")}','Open','Read','None'); Write-Host ready; Start-Sleep -Seconds 3600`;
    const child = spawn("powershell.exe", ["-NoProfile", "-Command", script], {
      stdio: ["ignore", "pipe", "ignore"],
      windowsHide: true,
    });
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`could not lock ${path}`));
    }, STEP_TIMEOUT_MS);
    child.stdout.on("data", (chunk) => {
      if (String(chunk).includes("ready")) {
        clearTimeout(timer);
        resolve(child);
      }
    });
    child.on("error", reject);
  });
const release = (child) =>
  new Promise((resolve) => {
    if (child.exitCode !== null) return resolve();
    child.once("exit", resolve);
    child.kill();
  });

test("cleaner: a file locked before the scan is listed under 'Left alone' with its reason and has no tick (QA A19)", async () => {
  expect(existsLong(fx.locked), `fixture missing ${fx.locked}`);
  await scanCleaner();
  expect(/Left alone\s*·\s*[1-9]/.test(await mainText()), "the review step lists nothing under 'Left alone'");
  const row = await evaluate(`(() => {
    const want = ${JSON.stringify(fx.locked.toLowerCase())};
    for (const li of document.querySelectorAll("main li")) {
      const [path, reason] = [...li.querySelectorAll("span[title]")].map((s) => s.title);
      if (path && path.toLowerCase().endsWith(want)) {
        return { reason: reason ?? "", ticks: li.querySelectorAll('input[type="checkbox"]').length };
      }
    }
    return null;
  })()`);
  expect(row, `no row for ${fx.locked} in the review step`);
  expect(row.reason.length > 0, "the locked file's row shows no reason");
  expect(row.ticks === 0, "the locked file's row has a checkbox");
  expect(existsLong(fx.locked), "the locked file disappeared");
});

test("cleaner: a file locked after the scan is reported with its reason, stays in place, and the rest moved (QA A19)", async () => {
  await scanCleaner();
  const locker = await holdExclusive(fx.tempOld);
  try {
    await click("main button", "Clean up");
    await click('[role="alertdialog"] button', "Clean up");
    await waitFor("button", "Open quarantine", null, SCAN_TIMEOUT_MS);
    const text = await mainText();
    expect(
      text.includes(basename(fx.tempOld)) && text.includes("in use by a running program"),
      `the done view does not list ${basename(fx.tempOld)} as in use:
${text}`,
    );
    expect(exists(fx.tempOld), "the locked file was moved");
    const held = await ledgerItems();
    expect(held.length > 0, "nothing was held after the clean");
    expect(
      !held.some((i) => i.originalPath.toLowerCase() === fx.tempOld.toLowerCase()),
      "the locked file is in the ledger",
    );
    // The done view prints the freed size, not a count: it must equal what the ledger holds.
    const freed = held.reduce((sum, i) => sum + i.sizeBytes, 0);
    expect(text.includes(`Freed ${formatBytes(freed)}`), `done view freed size != ledger total ${formatBytes(freed)}:
${text}`);
  } finally {
    await release(locker);
    await restoreAllHeld();
  }
});

test("quarantine: Stop during a batch restore keeps the rest held and returns to idle (QA A15)", async () => {
  try {
    // Earlier tests restore what they clean, so make our own batch when none is held.
    if ((await ledgerItems()).length < 2) {
      await scanCleaner();
      await click("main button", "Clean up");
      await click('[role="alertdialog"] button', "Clean up");
      await waitFor("main button", "Open quarantine", null, SCAN_TIMEOUT_MS);
    }
    const total = (await ledgerItems()).length;
    expect(total > 1, `need several held items to stop between, have ${total}`);
    await openDrawer();
    const outcome = await stopMidRun({
      within: DRAWER,
      start: `window.__qa.find("button", "Restore", ${q(DRAWER)}).click()`,
    });
    expect(outcome === "stopped", outcome);
    await waitFor("*", "Stopped: ", DRAWER, SCAN_TIMEOUT_MS);
    await sleep(SETTLE_MS);
    const left = (await ledgerItems()).length;
    expect(left > 0 && left < total, `expected some items still held after Stop, ${left} of ${total}`);
    expect(!(await has("button", "Stopping", DRAWER)), "Stop is stuck pending");
    expect(!(await has("*", "Restoring", DRAWER)), "the restore spinner is stuck");
    expect(await has("button", "Restore", DRAWER), "Restore is not offered again");
    // Idle again: the rest restores in one go and the ledger ends empty.
    await click("button", "Restore", DRAWER);
    await waitFor("*", "Restored ", DRAWER, SCAN_TIMEOUT_MS);
    await sleep(SETTLE_MS);
    expect((await ledgerItems()).length === 0, "items left after restoring the rest");
    expect(exists(fx.tempOld), "old temp sentinel not restored");
  } finally {
    await restoreAllHeld();
    await closeDrawer();
  }
});

test("cleaner: Stop during a clean keeps what moved and leaves the rest (QA A14)", async () => {
  try {
    await goTo("cleaner");
    if (await has("main button", "Scan again")) await click("main button", "Scan again");
    await click("button", "Scan for junk");
    await waitFor("main button", "Clean up", null, SCAN_TIMEOUT_MS);
    const outcome = await stopMidRun({
      within: "main",
      start: CLEAN_BUTTON,
      confirm: { sel: '[role="alertdialog"] button', text: "Clean up" },
    });
    expect(outcome === "stopped", outcome);
    await waitFor("main button", "Open quarantine", null, SCAN_TIMEOUT_MS);
    const text = await mainText();
    const m = /Stopped early: (\d+) moved to quarantine, (\d+) left/.exec(text);
    expect(m, `the done view does not say the run stopped early:
${text}`);
    const [moved, leftBehind] = [Number(m[1]), Number(m[2])];
    expect(moved > 0 && leftBehind > 0, `expected a partial run, got ${moved} moved / ${leftBehind} left`);
    const held = (await ledgerItems()).length;
    expect(held === moved, `ledger holds ${held} items but the view says ${moved} moved`);
    expect(!(await has("button", "Stopping")), "Stop is stuck pending");
    // Idle again: a fresh scan works and sees the items the stop left behind.
    await click("main button", "Scan again");
    await click("button", "Scan for junk");
    await waitFor("main button", "Clean up", null, SCAN_TIMEOUT_MS);
    const rescan = /(\d+) items selected/.exec(await mainText());
    expect(rescan && Number(rescan[1]) > 0, `the rescan found nothing, though Stop left ${leftBehind} items behind`);
  } finally {
    // Put everything back so later tests start from an empty ledger.
    await restoreAllHeld();
  }
  expect((await ledgerItems()).length === 0, "items left after restoring the partial clean");
  expect(exists(fx.tempOld), "old temp sentinel not restored");
});

test("quarantine: drawer shows the empty state once everything is restored", async () => {
  const left = await ledgerItems();
  expect(left.length === 0, `ledger still holds ${left.length} items`);
  await openDrawer();
  await waitFor("*", "Nothing is waiting in quarantine.", DRAWER);
  await closeDrawer();
});

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
  await waitFor('a[href="/settings"]', null, null, STARTUP_TIMEOUT_MS);

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
