# Module: cleaner (the flagship)

Purpose: find reclaimable junk, present it in 3 risk buckets with reasons, clean via quarantine.

## Scan targets (V1 rule packs — each is a `rules/*.json` file)
| Pack | Paths (expand env vars) | Default bucket |
|---|---|---|
| windows-temp | `%TEMP%`, `%WINDIR%\Temp` | Safe (files >24h old only) |
| recycle-bin | shell API (not raw `$Recycle.Bin` walk) | Safe |
| browser-cache | Chrome/Edge/Firefox/Brave cache dirs (NOT cookies/profiles) | Safe, skip if browser running → NeedsReview |
| thumbnail-icon-cache | `%LOCALAPPDATA%\Microsoft\Windows\Explorer\*.db` | Safe |
| windows-update-leftovers | `%WINDIR%\SoftwareDistribution\Download` | NeedsReview |
| crash-dumps-logs | `%LOCALAPPDATA%\CrashDumps`, `*.dmp`, `%WINDIR%\*.log` patterns | Safe/NeedsReview per rule |
| old-downloads | `%USERPROFILE%\Downloads`, untouched > 180 days | NeedsReview (never Safe) |
| installer-leftovers | `%TEMP%` `*.msi/*.exe` >30 days, `%WINDIR%\Downloaded Program Files` | NeedsReview |
| dev-caches (own pack, off by default) | npm/pip/cargo/gradle caches | NeedsReview |

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
