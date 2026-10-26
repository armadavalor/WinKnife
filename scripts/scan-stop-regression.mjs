/** Drive the real duplicates and treemap stores through Stop and rescan with fake IPC; nothing on disk is read. */
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
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

let listeners = [];
const cancels = [], scans = [];
globalThis.__scanApi = {
  cancelScan(id) { cancels.push(id); return Promise.resolve(true); },
  onScanProgress(handler) { listeners.push(handler); return Promise.resolve(() => { listeners = listeners.filter(h => h !== handler); }); },
  onExecProgress() { return Promise.resolve(() => {}); },
  startDuplicateScan() { const request = deferred(); scans.push(request); return request.promise; },
  treemapScan() { const request = deferred(); scans.push(request); return request.promise; },
  treemapCached() { return Promise.resolve(null); },
  treemapChildren() { return Promise.resolve(null); },
  treemapDrives() { return Promise.resolve([]); },
  execute() { return Promise.reject(new Error('not under test')); },
  setFolderKeepRule() { return Promise.reject(new Error('not under test')); },
  ignoreFolderGroup() { return Promise.reject(new Error('not under test')); },
  startSimilarImageScan() { const request = deferred(); scans.push(request); return request.promise; },
  setSimilarImageKeeper() { return Promise.resolve(imageUpdate); },
};
let imageUpdate = null;
const emit = (module, scanId, itemsScanned = 0) =>
  listeners.forEach(handler => handler({ module, scanId, itemsScanned, currentPath: 'C:\\big', pct: null, itemsTotal: null }));

const moduleUrl = source => 'data:text/javascript,' + encodeURIComponent(source);
const api = moduleUrl(`export const {${Object.keys(globalThis.__scanApi).join(',')}}=globalThis.__scanApi;`);
const types = moduleUrl('export const isApiError = e => e && typeof e.message === "string";');
const i18n = moduleUrl('export const t = key => key;');
function compileStore(file, siblings = {}) {
  let source = ts.transpileModule(
    readFileSync(new URL(`../src/lib/stores/${file}`, import.meta.url), 'utf8'),
    { compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext } },
  ).outputText;
  source = source.replaceAll('"$lib/api"', JSON.stringify(api)).replaceAll('"$lib/types"', JSON.stringify(types)).replaceAll('"$lib/i18n"', JSON.stringify(i18n));
  for (const [spec, url] of Object.entries(siblings)) source = source.replaceAll(JSON.stringify(spec), JSON.stringify(url));
  let code = compileModule(source, { filename: file.replace(/\.ts$/, '.js'), generate: 'client' }).js.code;
  code = code.replace(/from ['"](svelte[^'"]*)['"]/g, (_, spec) => 'from ' + JSON.stringify(pathToFileURL(require.resolve(spec)).href));
  return moduleUrl(code);
}
const execProgress = compileStore('execProgress.svelte.ts');
const { duplicatesStore } = await import(compileStore('duplicates.svelte.ts', { './execProgress.svelte': execProgress }));
const { treemapStore } = await import(compileStore('treemap.svelte.ts'));

// Duplicates: Stop pressed before the first event is sent when that event names the scan.
{
  const run = duplicatesStore.scan();
  await tick();
  await duplicatesStore.stop();
  assert.equal(cancels.length, 0, 'No id yet, so nothing to cancel');
  emit('duplicates', 'dup-1');
  assert.deepEqual(cancels, ['dup-1'], 'The first event must carry the deferred Stop');
  emit('duplicates', 'dup-1', 5);
  assert.equal(cancels.length, 1, 'One Stop, one cancel');
  scans.at(-1).resolve({ scanId: 'dup-1', wastedBytes: 0, scan: { sets: [], findings: [], linkedNames: [], skippedRoots: [], filesHashed: 0, unreadable: 0, cancelled: true }, folderGroups: [], recommendations: [] });
  await run;
  assert.equal(duplicatesStore.step, 'review');
  assert.equal(duplicatesStore.result.scan.cancelled, true);
  emit('duplicates', 'dup-1', 9);
  assert.equal(duplicatesStore.itemsScanned, 5, 'A late event from a finished scan is ignored');
  cancels.length = 0;
}

// Similar images (C18): nothing starts ticked, and a keeper change ticks nothing.
{
  const member = (path, keeper, ids) => ({ path, width: 9, height: 8, sizeBytes: 10, keeper, protected: false, similarity: 97, reason: 'r', findingIds: ids, recommendationIds: ids });
  const rec = (id, action) => ({ id, action, savingsBytes: 10 });
  duplicatesStore.mode = 'images';
  const run = duplicatesStore.scan();
  await tick();
  scans.at(-1).resolve({ scanId: 'img-1', groups: [{ id: 'g', members: [member('a', true, ['ra']), member('b', false, ['rb'])] }], recommendations: [rec('ra', 'infoOnly'), rec('rb', 'quarantine')], skippedRoots: [], unreadable: 0, cloudOnly: 0, cancelled: false });
  await run;
  assert.equal(duplicatesStore.step, 'review');
  assert.equal(duplicatesStore.selected.size, 0, 'Look-alikes start with nothing selected');
  const [keeper, other] = duplicatesStore.imageResult.groups[0].members;
  assert.equal(duplicatesStore.isImageSelectable(keeper), false, 'The keeper is never selectable');
  assert.equal(duplicatesStore.isImageSelectable(other), true);
  duplicatesStore.toggleFolder(['rb'], true);
  assert.equal(duplicatesStore.selectedBytes, 10);
  imageUpdate = { group: { id: 'g', members: [member('b', true, ['rb2']), member('a', false, ['ra2'])] }, replaced: ['ra', 'rb'], recommendations: [rec('rb2', 'infoOnly'), rec('ra2', 'quarantine')] };
  await duplicatesStore.chooseImageKeeper(duplicatesStore.imageResult.groups[0], other);
  assert.equal(duplicatesStore.selected.size, 0, 'A keeper change drops the old ticks and adds none');
  assert.equal(duplicatesStore.imageResult.recommendations.length, 2);
  duplicatesStore.reset();
  duplicatesStore.mode = 'files';
}

// Treemap: same early Stop, then a clean rescan that a late event cannot hijack.
const treeResult = (scanId, cancelled) => ({ scanId, summary: null, level: null, links: [], unreadable: [], cancelled });
treemapStore.selected = 'C:\\';
await treemapStore.listen(); // the page attaches this on mount
{
  const run = treemapStore.scan();
  await treemapStore.stop();
  assert.equal(cancels.length, 0, 'No id yet, so nothing to cancel');
  emit('treemap', 'tree-1');
  assert.deepEqual(cancels, ['tree-1'], 'A Stop pressed before the first event must not be lost');
  scans.at(-1).resolve(treeResult('tree-1', true));
  await run;
  assert.equal(treemapStore.partial, true);
  assert.equal(treemapStore.scanId, null);

  emit('treemap', 'tree-1', 42);
  assert.equal(treemapStore.scanId, null, 'A late event must not revive a finished scan id');
  assert.equal(treemapStore.scanned, 0);

  cancels.length = 0;
  const rescan = treemapStore.scan();
  assert.equal(treemapStore.scanId, null, 'A rescan starts without the old id');
  emit('treemap', 'tree-2', 3);
  assert.equal(cancels.length, 0, 'The earlier Stop does not carry over to a rescan');
  scans.at(-1).resolve(treeResult('tree-2', false));
  await rescan;
  assert.equal(treemapStore.partial, false, 'A completed rescan clears the stopped flag');
}
console.log('PASS: deferred Stop (duplicates, treemap), similar images start unselected, late events after the scan, rescan state reset');
delete globalThis.__scanApi;
