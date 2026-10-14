<!-- durum: aktif -->
# Phase 2 — Brain + duplicates

Prereq: Phase 1 accepted. Read ARCHITECTURE §4 fully, `modules/02-cleaner-core.md` (full), `modules/03-duplicate-finder.md`.

## Tasks
1. **Brain v1 scoring pipeline**: base score from rules + heuristic modifiers (age, size, locked, cache-dir) + bucket thresholds + `max_bucket` ceilings. Table-driven tests: ≥25 cases covering every modifier and gate.
2. **Full cleaner rule packs**: add browser-cache (with `owner_process_absent`), windows-update-leftovers, crash-dumps-logs, old-downloads, installer-leftovers, dev-caches (off by default). Fixture entries for each.
3. Cleaner UI: grouped-by-pack collapsible review, summary header ("X GB safely / Y GB review"), yellow deselected-by-default behavior, post-clean report.
4. **Duplicates module** per its spec (staged hashing, keeper heuristic, hardlink awareness, UI).
5. Settings page: per-module settings storage working (pack toggles, dup min-size, quarantine retention).

## Acceptance
- Scoring test table green; changing a rule's base score changes fixture buckets predictably.
- Duplicates: crafted-collision fixture zero false positives; cancellation mid-hash works.
- Full fixture profile scan: buckets match the fixture manifest exactly.

## Out of scope
LLM layer, treemap, startup.
