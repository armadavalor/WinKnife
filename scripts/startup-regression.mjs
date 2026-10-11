/** Exercise the real Svelte store with deferred IPC responses; no OS startup entries change. */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';

const require = createRequire(new URL('../package.json', import.meta.url));
const { compileModule } = require('svelte/compiler');
const ts = require('typescript');
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const scans = [], mutations = [];
globalThis.__startupApi = {
  startupScan() { const request = deferred(); scans.push(request); return request.promise; },
  startupEnable() { const request = deferred(); mutations.push(request); return request.promise; },
  execute() { const request = deferred(); mutations.push(request); return request.promise; },
};
const moduleUrl = source => 'data:text/javascript,' + encodeURIComponent(source);
const api = moduleUrl('export const {startupScan,startupEnable,execute}=globalThis.__startupApi;');
const types = moduleUrl('export const isApiError = e => e && typeof e.message === "string";');
let source = ts.transpileModule(
  readFileSync(new URL('../src/lib/stores/startup.svelte.ts', import.meta.url), 'utf8'),
  { compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext } },
).outputText;
source = source.replaceAll('"$lib/api"', JSON.stringify(api)).replaceAll('"$lib/types"', JSON.stringify(types));
// The store's one sibling import is pure TypeScript, which node loads as is.
const execNotes = new URL('../src/lib/stores/execNotes.ts', import.meta.url).href;
source = source.replaceAll('"./execNotes"', JSON.stringify(execNotes));
let code = compileModule(source, { filename: 'startup.svelte.js', generate: 'client' }).js.code;
code = code.replace(/from ['"](svelte[^'"]*)['"]/g, (_, spec) => 'from ' + JSON.stringify(pathToFileURL(require.resolve(spec)).href));
const { startupStore: store } = await import(moduleUrl(code));
const entry = { id: 'entry', source: 'run', key: 'example', enabled: true, os_owned: false };
const result = id => ({ scanId: id, scan: { entries: [entry], unreadable: [] }, recommendations: [{ id: 'recommendation', justification: { kind: 'startupEntry', key: entry.key } }] });

const initial = [store.load(), store.reload(), store.reload()];
assert.equal(scans.length, 1, 'Mount and concurrent Refresh must share an enumeration');
await store.enable(entry);
assert.equal(mutations.length, 0, 'No mutation can start while enumeration is pending');
scans[0].resolve(result('initial'));
await Promise.all(initial);
assert.equal(store.status, 'ready');

for (const action of ['enable', 'disable']) {
  const scanCount = scans.length, changeCount = mutations.length;
  const change = store[action](entry);
  await store.reload();
  await store.enable(entry);
  await store.disable(entry);
  assert.equal(scans.length, scanCount, 'Refresh must not start before the mutation finishes');
  assert.equal(mutations.length, changeCount + 1, 'Only one mutation may run');
  // A re-enable resolves to its move warnings; a disable to an execute report.
  mutations.at(-1).resolve(action === 'enable' ? [] : { items: [] });
  await Promise.resolve();
  assert.equal(scans.length, scanCount + 1, 'The mutation must request fresh data');
  await store.reload();
  assert.equal(scans.length, scanCount + 1, 'Refresh must not race the final enumeration');
  scans.at(-1).resolve(result(action));
  await change;
  assert.equal(store.busy, null);
  assert.equal(store.status, 'ready');
}
const warned = store.enable(entry);
mutations.at(-1).resolve([{ path: 'Vendor.lnk', reason: 'both kept' }]);
await Promise.resolve();
scans.at(-1).resolve(result('warned'));
await warned;
assert.equal(store.lastReport, 'Vendor.lnk: both kept', 'A re-enable that came back only in part must say so');
const failure = store.reload();
scans.at(-1).reject(new Error('Enumeration failed'));
await failure;
assert.equal(store.status, 'error');
const retry = store.load();
scans.at(-1).resolve(result('retry'));
await retry;
assert.equal(store.status, 'ready', 'A failed refresh must not leave the store locked');
console.log('PASS: mount/refresh coalescing, enable/disable races, duplicate mutations, re-enable warnings, failure recovery');
delete globalThis.__startupApi;
