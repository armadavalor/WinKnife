# Polish QA — 2026-09-13

## Findings

| Item | Result and evidence |
| --- | --- |
| Startup Refresh race | Fixed. Concurrent mount/Refresh shares one enumeration. Mutations reject another mutation or Refresh while busy and perform their own final enumeration. The deferred-response regression test covers enable, disable, duplicate mutations and failed enumeration recovery. The original implementation failed the concurrent-refresh assertion (2 scans instead of 1). |
| Cleaner previous scan | Fixed. The idle page reads existing persisted history through dashboardSummary. It shows date, item count and estimated bytes, including cancelled status. It never rehydrates actionable file findings. Returning to idle refreshes the history. Before the change the QA dashboard showed a scan while cleaner showed no history; afterwards the same saved record appeared on cleaner. |
| IPC fallback warning | Not reproduced in the production build using the Tauri custom protocol, a QA identifier and the embedded frontend. Reload and seven route transitions produced no console messages, exceptions or failed network requests. Evidence applies to this WebView2/Windows environment; no transport workaround was added. |
| Horizontal overflow | Not reproduced at 1100×720 and 880×560 on seven routes in release. Main scrollWidth equalled clientWidth. A separate dev UI-only fixture with 30 long file paths also fit at 880px. No blanket overflow-hiding CSS was added. Populated duplicates, every dialog/drawer combination and other display scaling settings still need fresh-VM acceptance. |

## Checks

- Svelte/TypeScript: zero errors, zero warnings.
- UI: 12 treemap tests plus the deferred startup regression scenarios pass.
- Clippy: all targets pass with warnings denied.
- Rust unit run without opt-outs: **481 passed; 5 failed because this Windows
  session cannot create symlinks**. These are outstanding safety checks, not
  application regressions or successful tests.
- A second local run with the project's documented WINKNIFE_ALLOW_SKIP_LINK_TESTS=1
  completed the remaining unit/integration/doc suites. The five link checks
  explicitly asserted nothing; three existing ignored doc examples stayed ignored.
- QA production build succeeded after selectively rebuilding dependency caches
  whose build-script output still referenced the old Cleanify checkout path.

Release requires the five link checks to run for real in Developer Mode or an
elevated Windows session, plus the fresh-VM checklist in RELEASING.md.

## Repeating desktop QA safely

Do not assume `--dry-run` protects files: it replaces the power controller only.
Startup retention purge happens before the UI opens. APPDATA environment overrides
also do not change Tauri's native Windows known-folder lookup.

Build with a separate identifier via a temporary Tauri config, for example
`{"identifier":"com.winknife.qa"}`. For production-mode QA use the Tauri build
command with `--no-bundle --config <path>`; a plain debug build does not test
the embedded production frontend/custom protocol.

Launch that QA binary with disposable USERPROFILE, LOCALAPPDATA and TEMP paths
under one fixture root. Leave compiler/rustup environment paths unchanged during
the build. Confirm the startup log actually names **com.winknife.qa** before any
interaction. Its native database still resides under Windows roaming app data.

Set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS to a local remote-debugging port for
the QA session. Close the dev QA instance before opening a production instance
with the same identifier; WebView2 can reuse a process and its original port.
After HMR, reload before importing stores for UI-only fixture injection; otherwise
a different module instance can make a test appear to run without changing the UI.

The September 13 CDP probes and JSON evidence are local scratch artifacts at
T:/ChatGPT/winknife-qa. They are not a portable CI test suite. Fixtures injected
for layout must never be submitted for execution; they only test rendering.

## Next work, in agreed order

1. Finish remaining polish verification in a fresh VM; no speculative CSS fix.
2. Complete release acceptance and explicitly decide the remaining Phase 4 LLM
   scope. Packaging/README preparation is not full Phase 4 acceptance.
3. Graphify knowledge graph, then only useful additional repo/skill integrations.
   Avoid paid API dependencies and unnecessary background hooks.
