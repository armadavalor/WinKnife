# Module Backlog (post-V1, branch-and-grow)

Rule for admission: solves a real, recurring Windows annoyance; fits scan→review→execute or a single obvious screen; no settings jungle. Each entry below is a seed spec — promote to a full `NN-name.md` before implementation.

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
- **Sticky notes / desktop widgets / quick notes** — resident desktop surfaces violate the lightweight promise and the toolbox identity; Windows + dedicated apps already cover this.
- **Git helpers / localhost utilities** — vague scope, no single-screen fit; IDE and CLI territory.
- **Standalone kill-process** — covered inside port-inspector and file-unlocker.
- Already covered elsewhere, no new entries needed: power timers (module 01), startup apps (module 05), temp/cache cleaning (module 02), large-file/disk-usage views (module 04 treemap), clipboard history (Tier B), Ollama/local-LLM support (ARCHITECTURE §4), portable exe + dark-first UI (PLAN).

## Positioning notes (PocketKnife draft, keep for Phase 4 README)
"One app. Many tools. Zero bloat." / "Completes Windows rather than replacing it." / Long-term ambition: first app installed after a fresh Windows setup. "PocketKnife" is also on the PLAN §7 name shortlist.
