// Drives an installed WinKnife over the WebView2 DevTools protocol and records
// the frames for the README demo GIF (docs/DEMO-SCRIPT.md, GIF 1).
//
// The app must run with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS set to
// --remote-debugging-port=9222. Runs on a throwaway CI profile only
// (.github/workflows/demo.yml); never point it at a real user profile,
// because it really cleans: safe items move to quarantine, then one batch is
// restored.
//
// Usage: node scripts/demo/record.mjs <out-dir>
// Output: <out-dir>/NNNNN.png plus frames.txt (ffmpeg concat list with the
// real duration of each frame; waits are cut to CUT_HOLD_S).

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const PORT = 9222;
const WIDTH = 1280;
const HEIGHT = 800;
const CURSOR_MOVE_MS = 650;
const CUT_HOLD_S = 0.25;
const MAX_FRAME_S = 0.5;
const STARTUP_TIMEOUT_MS = 90_000;
const STEP_TIMEOUT_MS = 180_000;

const outDir = process.argv[2] ?? "frames";
mkdirSync(outDir, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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
      // DevTools endpoint not up yet.
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

// In-page helpers: element lookup by selector + visible text, and a fake
// cursor (CDP screenshots do not include the OS pointer).
const HELPERS = `
window.__demo = {
  find(sel, text, within) {
    const root = within ? document.querySelector(within) : document;
    if (!root) return null;
    const all = [...root.querySelectorAll(sel)].filter((el) => {
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && (!text || el.textContent.trim().includes(text));
    });
    return all[0] ?? null;
  },
  center(sel, text, within) {
    const el = this.find(sel, text, within);
    if (!el) return null;
    el.scrollIntoView({ block: "nearest" });
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
  },
  cursor() {
    let c = document.getElementById("__demo-cursor");
    if (c) return c;
    c = document.createElement("div");
    c.id = "__demo-cursor";
    c.innerHTML = '<svg width="22" height="28" viewBox="0 0 22 28"><path d="M2 2 L2 22 L7 17 L11 26 L15 24 L11 15 L18 15 Z" fill="#fff" stroke="#111" stroke-width="1.6" stroke-linejoin="round"/></svg>';
    Object.assign(c.style, {
      position: "fixed", left: "640px", top: "400px", zIndex: 2147483647,
      pointerEvents: "none", transition: "left ${CURSOR_MOVE_MS}ms ease-in-out, top ${CURSOR_MOVE_MS}ms ease-in-out",
      filter: "drop-shadow(0 1px 2px rgba(0,0,0,.5))",
    });
    document.body.appendChild(c);
    return c;
  },
  moveTo(x, y) {
    const c = this.cursor();
    c.style.left = x - 2 + "px";
    c.style.top = y - 2 + "px";
  },
  ripple(x, y) {
    const d = document.createElement("div");
    Object.assign(d.style, {
      position: "fixed", left: x - 14 + "px", top: y - 14 + "px", width: "28px", height: "28px",
      borderRadius: "50%", border: "2px solid rgba(255,255,255,.85)", zIndex: 2147483646,
      pointerEvents: "none", transition: "transform 350ms ease-out, opacity 350ms ease-out",
    });
    document.body.appendChild(d);
    requestAnimationFrame(() => { d.style.transform = "scale(1.8)"; d.style.opacity = "0"; });
    setTimeout(() => d.remove(), 400);
  },
};
true;
`;

const q = (v) => JSON.stringify(v ?? null);

async function waitFor(sel, text, within, timeout = STEP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const found = await evaluate(`!!window.__demo?.find(${q(sel)}, ${q(text)}, ${q(within)})`);
    if (found) return;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${sel} "${text ?? ""}"`);
}

async function pointAt(sel, text, within) {
  await waitFor(sel, text, within);
  const pos = await evaluate(`window.__demo.center(${q(sel)}, ${q(text)}, ${q(within)})`);
  if (!pos) throw new Error(`no element ${sel} "${text ?? ""}"`);
  await evaluate(`window.__demo.moveTo(${pos.x}, ${pos.y})`);
  await sleep(CURSOR_MOVE_MS + 150);
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: pos.x, y: pos.y });
  return pos;
}

async function click(sel, text, within) {
  const pos = await pointAt(sel, text, within);
  await sleep(350);
  await evaluate(`window.__demo.ripple(${pos.x}, ${pos.y})`);
  for (const type of ["mousePressed", "mouseReleased"]) {
    await cdp.send("Input.dispatchMouseEvent", { type, x: pos.x, y: pos.y, button: "left", clickCount: 1 });
  }
}

// Frame capture runs alongside the script. `cut()` drops the frames of a long
// wait (scan progress, the move to quarantine) and keeps a short hold instead.
const frames = [];
let recording = false;
let capturing = true;

async function captureLoop() {
  while (capturing) {
    if (!recording) {
      await sleep(50);
      continue;
    }
    const t = Date.now();
    const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
    if (!recording) continue;
    const name = String(frames.length).padStart(5, "0") + ".png";
    writeFileSync(join(outDir, name), Buffer.from(data, "base64"));
    frames.push({ name, t, hold: null });
  }
}

async function cutDuring(work) {
  recording = false;
  if (frames.length) frames[frames.length - 1].hold = CUT_HOLD_S;
  await work();
  recording = true;
}

function writeConcatList() {
  const lines = [];
  frames.forEach((f, i) => {
    const next = frames[i + 1];
    const gap = next ? (next.t - f.t) / 1000 : 1.5;
    const duration = f.hold ?? Math.min(gap, MAX_FRAME_S);
    lines.push(`file '${f.name}'`, `duration ${duration.toFixed(3)}`);
  });
  // The concat demuxer ignores the last duration unless the file repeats.
  lines.push(`file '${frames[frames.length - 1].name}'`);
  writeFileSync(join(outDir, "frames.txt"), lines.join("\n") + "\n");
}

async function main() {
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: WIDTH,
    height: HEIGHT,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await evaluate(HELPERS);
  await waitFor('a[href="/settings"]');
  // The sidebar groups tools in accordions; start on the cleaner's tab.
  if (!(await evaluate(`!!window.__demo.find('a[href="/cleaner"]')`))) {
    await click('nav button[data-tools~="cleaner"]');
  }
  await click('a[href="/dashboard"]');
  await sleep(1500);
  await evaluate("window.__demo.moveTo(900, 520)");
  await sleep(CURSOR_MOVE_MS);

  const loop = captureLoop();
  recording = true;

  // 1. Dashboard, hold still.
  await sleep(1800);

  // 2. Cleaner → Scan; keep the start of the progress, cut the rest.
  await click('a[href="/cleaner"]');
  await sleep(900);
  await click("button", "Scan for junk");
  await sleep(1400);
  await cutDuring(() => waitFor("button", "Clean up", "main"));
  await sleep(1200);

  // 3. Open one safe item's reason, then point at a needs-review item.
  await pointAt('[aria-controls^="group-safe-"]');
  await sleep(700);
  await click('[id^="group-safe-"] button[aria-expanded]');
  await sleep(2200);
  await pointAt('[id^="group-needsReview-"] button[aria-expanded]');
  await sleep(1800);

  // 4. Clean only what is ticked by default (the safe groups), confirm.
  await click("main button", "Clean up");
  await sleep(1300);
  await click('[role="alertdialog"] button', "Clean up");
  await cutDuring(() => waitFor("button", "Open quarantine"));
  await sleep(1500);

  // 5. Quarantine drawer lists the batch with its date.
  await click("button", "Open quarantine");
  await sleep(2000);

  // 6. Restore → "Restored N items."
  await click("button", "Restore", 'aside[aria-label="Quarantine"]');
  await cutDuring(() => waitFor("*", "Restored ", 'aside[aria-label="Quarantine"]', 60_000));
  await sleep(1800);

  recording = false;
  capturing = false;
  await loop;
  writeConcatList();
  console.log(`recorded ${frames.length} frames into ${outDir}`);
}

try {
  await main();
} catch (err) {
  // Leave a still of the failing screen for the workflow artifact.
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(outDir, "failure.png"), Buffer.from(data, "base64"));
  console.error(err);
  process.exitCode = 1;
} finally {
  capturing = false;
  recording = false;
  cdp.close();
}
