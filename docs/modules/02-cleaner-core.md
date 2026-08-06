# Module: cleaner (the flagship)

Purpose: find reclaimable junk, present it in 3 risk buckets with reasons, clean via quarantine.

## Scan targets (V1 rule packs — each is a `rules/*.json` file)
| Pack | Paths (expand env vars) | Default bucket |
|---|---|---|
| windows-temp | `%TEMP%`, `%WINDIR%\Temp` | Safe (files >24h old only) |
| recycle-bin | shell API (not raw `$Recycle.Bin` walk) | Safe |
| browser-cache | Chrome/Edge/Firefox/Brave cache dirs (NOT cookies/profiles) | Safe, skip if browser running → NeedsReview |
| thumbnail-icon-cache | `%LOCALAPPDATA%\Microsoft\Windows\Explorer\*.db` | Safe |
| windows-update-leftovers (off by default) | `%WINDIR%\SoftwareDistribution\Download` | InfoOnly (see below) |
| crash-dumps-logs | `%LOCALAPPDATA%\CrashDumps`, `*.dmp` patterns | Safe/NeedsReview per rule |
| old-downloads | `%USERPROFILE%\Downloads`, untouched > 180 days | NeedsReview (never Safe) |
| installer-leftovers | `%TEMP%` `*.msi/*.exe` >30 days | NeedsReview |
| dev-caches (own pack, off by default) | npm/pip/cargo/gradle caches | NeedsReview |

**Why `windows-update-leftovers` is InfoOnly, and why two `%WINDIR%` locations are gone.**
`%WINDIR%` is a tree-scoped protected path, and CLAUDE.md's hard rule is that a protected path can never receive a Safe rating *or* a delete action. A NeedsReview row there would have been a button this app must never render, so the pack ships `action: info_only` with `max_bucket: do_not_delete` — it can report the location and is structurally incapable of proposing anything about it. That is not a workaround for the rule; the red bucket being informational is the honesty feature, and this pack is its first real case. Its `reason` therefore has to earn the row: name the space involved and point at the tool that *will* clear it.

The two other `%WINDIR%` locations this table used to name — `%WINDIR%\*.log` (in `crash-dumps-logs`) and `%WINDIR%\Downloaded Program Files` (in `installer-leftovers`) — are deliberately **not shipped**. They sit in the same protected tree, so they could only ever be informational too, and one informational rule makes the point where three would be noise pretending to be coverage. Both packs cover their profile-scoped locations instead.

`%WINDIR%` gets **no carve-out**, now or later. A documented exception is how a protected-path list erodes: the mechanism outlives the justification, and the next contributor argues their directory is just as safe. (Architect resolution, `docs/DECISIONS-NEEDED.md` — phase 2.)

Red bucket (DoNotDelete) is **produced, not targeted**: anything the walker surfaces that hits protected paths, locked files, unknown-but-system-adjacent locations. Shown with reason, no action button — this is the honesty feature.

## Flow
1. `scan()`: walk pack paths with shared scanner (jwalk), collect Findings, tag with matched rule id. Recycle bin via `SHQueryRecycleBin`/`SHEmptyRecycleBin` wrappers — separate finding kind, size from shell API.
2. `analyze()`: brain scores per ARCHITECTURE §4. Group recommendations by pack for the UI (collapsible groups with per-group select-all).
3. `execute()`: approved → executor → quarantine batch (recycle bin empty is the one non-quarantine action; label it clearly in confirm dialog).

## UI specifics
- Summary header after scan: "≈ X GB reclaimable safely, Y GB needs your review".
- Each item row: icon, path (middle-truncated, full on hover), size, age, `reason` line.
- Green bucket: selected by default. Yellow: deselected by default. Red: no checkboxes.
- Post-clean report: reclaimed GB + "restore" link → quarantine drawer.

## Safety details
- Skip files modified < 1 hour ago even in Safe packs (in-use heuristic).
- Locked file → per-item skip + report; batch continues.
- Browser-running detection: process name check before offering browser packs as Safe.

## Acceptance
- Fixture scan produces deterministic buckets (table-driven fixture manifest asserts each expected path→bucket).
- Quarantine round-trip: clean fixture, restore batch, byte-identical tree.
- Protected-path injection test: plant `C:\Windows`-mimicking path in fixture root config → must land in red regardless of rules.
