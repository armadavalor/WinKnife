<!-- durum: aktif -->
# Module Backlog (post-V1, branch-and-grow)

Rule for admission: solves a real, recurring Windows annoyance; fits scan→review→execute or a single obvious screen; no settings jungle. Each entry below is a seed spec — promote to a full `NN-name.md` before implementation.

> Scheduling now lives in [docs/ROADMAP.md](../ROADMAP.md); this file keeps seed specs and non-goals.

## Tier A — natural next (V1.x)
- **empty-folder-hunter** — trivial on top of shared scanner; quarantine-safe; good first issue material.
- **file-unlocker** ("who is locking this file?") — Restart Manager API (`RmGetList`) to name the process; offer close-handle guidance (not force-kill by default). High daily-pain value.
- **context-menu-cleaner** — enumerate shell extension keys (`*\shellex`, `Directory\shell`...), disable via rename-prefix (reversible). Known-entries rule pack.
- **big-file-archiver** — from treemap data: cold large files (>1 GB, unopened >1 yr) → suggest move to chosen archive drive; move-with-verify, DB record, "leave breadcrumb .txt" option.

## Tier B — strong candidates (V2)
- **uninstaller+** — list installed apps (registry + winget), leftover-sweep after uninstall (reuse cleaner engine with vendor-path rules).
- **debloat-lite** — curated toggles for Windows annoyances (web results in Start, lock-screen ads, telemetry basics). Only documented, reversible registry toggles; each with source link. Politically sensitive — keep conservative.
- **clipboard-history+** — persistent searchable clipboard (opt-in, local only, excludes password-manager sources).
- **screenshot-organizer** — watch folder, rename by content date, dedupe burst shots (reuses duplicate hasher).
- **network-monitor-lite** — "what is using my internet right now" per-process table (GetExtendedTcpTable + perf counters). Read-only.
- **env-var-editor** — Windows' env-var dialog is a 1995 relic. Searchable user/system variable editor with PATH split into rows, snapshot-before-edit (restorable), diff preview on save. Single screen; registry writes behind the same reversible-executor discipline.
- **port-inspector** — "what is squatting on :3000?" — listening ports table with owning process, one-click graceful close (force-kill behind confirm). Merge candidate with network-monitor-lite into one Network screen.
- **hosts-editor** — hosts file editor with syntax validation, per-block enable/disable toggles, automatic timestamped backup + restore. Absorbs the Tier C "hosts-file ad-block toggle" idea.

## Tier C — ideas parking lot
- Batch renamer with live preview; system-info/health snapshot exporter; scheduled auto-clean profiles ("every Sunday, green bucket only"); portable app updater; "PC hygiene score" gamification on dashboard (careful: keep honest, not nagware).
- **net-toolbox** — one screen of documented network fixes: DNS flush, IP release/renew, winsock reset. Each button shows the exact command it runs and what it does; winsock reset behind an extra confirm. Merge candidate with the Network screen (port-inspector + network-monitor-lite).
- **services-lite** — curated known-safe Windows service toggles with source links. Same conservative rules as debloat-lite; likely merges into it rather than shipping separately.
- Community plugin system / marketplace — post-V2 at the earliest. The `ToolModule` trait registry is already the seed; do not design a plugin ABI before the trait has survived several real modules.

## Explicit non-goals (do not build)
Registry "optimizer"/defrag (snake oil — undermines trust positioning), driver updater (risk), RAM "booster" (placebo), antivirus features (out of depth). README should proudly list these as non-goals; it's part of the honesty brand.

## Reviewed and rejected (from the PocketKnife draft, 2026-08-06 — do not re-add without new argument)
- ~~**Sticky notes / desktop widgets / quick notes**~~ — rejection lifted for sticky notes on 2026-10-03 (owner request, three-model brainstorm): scheduled for v0.2.0 in [ROADMAP](../ROADMAP.md) as an optional tool with its own trash. Desktop widgets stay rejected.
- **Git helpers / localhost utilities** — vague scope, no single-screen fit; IDE and CLI territory.
- **Standalone kill-process** — covered inside port-inspector and file-unlocker.
- Already covered elsewhere, no new entries needed: power timers (module 01), startup apps (module 05), temp/cache cleaning (module 02), large-file/disk-usage views (module 04 treemap), clipboard history (Tier B), Ollama/local-LLM support (ARCHITECTURE §4), portable exe + dark-first UI (PLAN).

## Positioning notes (PocketKnife draft, keep for Phase 4 README)
"One app. Many tools. Zero bloat." / "Completes Windows rather than replacing it." / Long-term ambition: first app installed after a fresh Windows setup. "PocketKnife" is also on the PLAN §7 name shortlist.

## Polish queue — user notes (2026-10-18)
Not modules: fixes and polish to existing screens, in the user's order. Items 1, 2 and 4 shipped in #50 (2026-10-18).

1. **Defender at 85–90% CPU during a scan.** Cause: the lock probe opened every file for read, which Defender scans on access. *Fixed in #50* (probe asks for `DELETE`, not read); user confirmed CPU dropped.
2. **Huge dropdown arrows in the findings list.** Cause: a stray `` byte as the Icon class (from f793205) dropped its size. *Fixed* (FindingList and Duplicates).
3. **Direct delete without quarantine.** Conflicts with the hard rule "never permanently delete user files" — user chose quarantine-only for now (K-016); revisit after the faster clean. Item 4's fix removes most of the speed reason.
4. **Clean of 20 GB junk took 12+ minutes.** Not the purge: the clean moved 84,919 files into quarantine at ~8.5 ms each, ~8 ms of it the same read-open probe. *Fixed* (~0.1 ms per probe). Purge itself is untouched; it runs one autocommit DELETE per row — batch in a transaction if it shows up slow.
5. **Delete/clean progress shows no file names and the bar looks frozen.** Show the current path and count; give running bars a small motion (indeterminate shimmer / stripe) so progress reads as alive. *Fixed* (file names and counts came with #39; the motion: sliding segment for unknown totals, drifting stripe on running bars, off under `prefers-reduced-motion` and `data-animations="off"`).
6. **Logo animation.** The knife opening/closing on loading screens and on logo hover.
7. **Background light.** Stronger, wave-like moving light; at least 3 variants to pick from; reacts to the mouse.
8. **Animations off switch.** On by default, one toggle to turn off; off automatically under `prefers-reduced-motion` and in a performance mode. Items 5–7 must honour it.
9. **QA/QC pass over everything.** User: still much to fix. Priority: delete / clean / quarantine (restore, purge, locked and long paths, cancel mid-run) and search / scan (Stop, rescans, large trees, the flaky e2e `duplicates: Stop returns to idle`). Write the cases down as a checklist first, then fix with failing tests.
