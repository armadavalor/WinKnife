<!-- durum: aktif -->
# Phase 1 — First blood (shutdown timer + green-bucket cleaner + quarantine)

Prereq: Phase 0 accepted. Read `modules/01-shutdown-timer.md`, `modules/02-cleaner-core.md` (green scope only), ARCHITECTURE §4 (gates only), §6.

## Tasks
1. **Quarantine service** (build FIRST — everything depends on it): move/copy-verify semantics, DB records, retention purge, restore, size cap. Tempdir round-trip tests.
2. **Executor**: takes approved recommendations → quarantine batch → `ExecReport`; per-item error isolation; tracing log per item.
3. **Shutdown timer module** per its spec (incl. tray, `--dry-run` flag, persistence).
4. **Brain v0**: hard gates only — `protected_paths()` + locked-file check + bucket types. No scoring yet: rule → fixed bucket.
5. **Cleaner v0**: rule packs `windows-temp`, `recycle-bin`, `thumbnail-icon-cache` only (all-green scope); rules parser + `RULES-FORMAT.md` schema + validate-all-packs test; scan→review→execute UI flow with the 3-step pattern (yellow/red buckets render but will be empty except gate-produced reds).
6. Quarantine drawer UI: list batches, restore, purge-now.

## Acceptance
- Fixture scan → deterministic findings; clean → files in quarantine + DB rows; restore → byte-identical.
- Protected-path fixture lands red. Locked-file fixture skipped with per-item report.
- Shutdown timer acceptance items from its spec pass.
- A stranger can build from README dev-setup section (add it).

## Out of scope
Scoring/heuristics, yellow-bucket packs, LLM, duplicates, treemap, startup.
