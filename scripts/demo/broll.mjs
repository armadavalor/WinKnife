// Takes the 2x stills for the README b-roll (demo/, Remotion) from an installed
// WinKnife over the WebView2 DevTools protocol. Same rules as record.mjs:
// throwaway CI profile only (.github/workflows/demo.yml), because it really
// cleans the seeded junk into quarantine.
//
// Usage: node scripts/demo/broll.mjs <out-dir>
// Output: <out-dir>/<shot>.png (2560x1600).

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const PORT = 9222;
const WIDTH = 1280;
const HEIGHT = 800;
const SCALE = 2;
const STARTUP_TIMEOUT_MS = 90_000;
const STEP_TIMEOUT_MS = 180_000;
const SETTLE_MS = 1500;
const DEMO_DRIVE = process.env.DEMO_DRIVE ?? "W:\\";
const TOUR = [
  "startup", "duplicates", "empty-folders", "hosts", "tweaks", "uninstaller", "apps", "network",
  "env-vars", "context-menu", "scheduled-scan", "archiver", "file-unlocker", "repair",
  "restore-point", "keep-awake", "shutdown-timer", "legacy-panels", "settings",
];

const outDir = process.argv[2] ?? "stills";
mkdirSync(outDir, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function pageTarget() {
  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = list.find((t) => t.type === "page" && !t.url.startsWith("about:"));
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // DevTools endpoint not up yet.
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

const cdp = await connect(await pageTarget());

async function evaluate(expression) {
  const res = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (res.exceptionDetails) throw new Error(res.exceptionDetails.text + ": " + expression);
  return res.result.value;
}

const q = (v) => JSON.stringify(v ?? null);
// First visible element matching a selector and (optionally) its text.
const findExpr = (sel, text, within) => `(() => {
  const root = ${within ? `document.querySelector(${q(within)})` : "document"};
  if (!root) return null;
  return [...root.querySelectorAll(${q(sel)})].find((el) => {
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && (!${q(text)} || el.textContent.trim().includes(${q(text)}));
  }) ?? null;
})()`;

async function has(sel, text, within) {
  return evaluate(`!!${findExpr(sel, text, within)}`);
}

async function waitFor(sel, text, within, timeout = STEP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await has(sel, text, within)) return;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${sel} "${text ?? ""}"`);
}

async function click(sel, text, within) {
  await waitFor(sel, text, within);
  await evaluate(`${findExpr(sel, text, within)}.click()`);
}

async function goTo(route) {
  const tab = `nav button[data-tools~="${route}"]`;
  if (!(await has(`nav a[href="/${route}"]`)) && (await has(tab))) await click(tab);
  await click(`nav a[href="/${route}"]`);
  await sleep(SETTLE_MS);
}

async function shot(name) {
  await sleep(600);
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(outDir, `${name}.png`), Buffer.from(data, "base64"));
  console.log(`shot ${name}`);
}

async function main() {
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: WIDTH,
    height: HEIGHT,
    deviceScaleFactor: SCALE,
    mobile: false,
  });
  await waitFor('a[href="/settings"]');
  await goTo("dashboard");
  await sleep(2500);
  await shot("01-dashboard");

  await goTo("cleaner");
  await click("button", "Scan for junk");
  await waitFor("button", "Clean up", "main");
  await sleep(SETTLE_MS);
  await click('[id^="group-safe-"] button[aria-expanded]');
  await shot("02-cleaner");

  await click("main button", "Clean up");
  await click('[role="alertdialog"] button', "Clean up");
  await waitFor("button", "Open quarantine");
  await click("button", "Open quarantine");
  await sleep(SETTLE_MS);
  await shot("03-quarantine");
  await click("button", "Restore", 'aside[aria-label="Quarantine"]');
  await waitFor("*", "Restored ", 'aside[aria-label="Quarantine"]', 60_000);
  await shot("04-restored");
  await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });

  await goTo("startup");
  await waitFor("button", "Refresh", "main");
  await sleep(SETTLE_MS);
  await shot("05-startup");

  await goTo("treemap");
  await waitFor("main select");
  await evaluate(`(() => {
    const s = document.querySelector("main select");
    s.value = ${q(DEMO_DRIVE)};
    s.dispatchEvent(new Event("change", { bubbles: true }));
  })()`);
  await sleep(SETTLE_MS);
  await click("main button.btn-primary");
  await waitFor("main canvas");
  await waitFor("main button.btn-primary");
  await sleep(2500);
  await shot("06-treemap");

  // One still per tool for the fast tour; a page that will not open is skipped.
  for (const route of TOUR) {
    try {
      const tab = `nav button[data-tools~="${route}"]`;
      if (!(await has(`nav a[href="/${route}"]`)) && (await has(tab))) await click(tab);
      if (!(await has(`nav a[href="/${route}"]`))) throw new Error("not in the sidebar");
      await goTo(route);
      await sleep(1500);
      await shot(`tour-${route}`);
    } catch (err) {
      console.error(`tour ${route}: ${err.message}`);
    }
  }
}

try {
  await main();
} catch (err) {
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(outDir, "failure.png"), Buffer.from(data, "base64"));
  console.error(err);
  process.exitCode = 1;
} finally {
  cdp.close();
}
