# Phase 3 — Treemap + startup + polish

Prereq: Phase 2 accepted. Read `modules/04-treemap.md`, `modules/05-startup-manager.md`.

## Tasks
1. **Treemap module** per spec: lazy `get_children` backend, canvas squarified renderer, zoom/breadcrumb, category colors, cross-link actions to cleaner/duplicates (pre-filled path routing).
2. **Startup module** per spec: 3 sources, StartupApproved-compatible disable, signature check, `startup-known.json` pack, impact tiers.
3. **Dashboard becomes real**: cards per module (last scan, reclaimable estimate, active shutdown timer), quick actions.
4. Polish pass: loading/empty/error states for every route; keyboard navigation on lists; confirm-dialog copy review (calm, honest tone — no scare tactics); app icon; window state persistence.
5. Perf check: 100k-file synthetic tree treemap first-paint < 3s; scans stay cancellable everywhere.

## Acceptance
Module specs' acceptance lists pass; dashboard reflects real data; a full click-through of every route finds no dead ends or unstyled states.

## Out of scope
LLM, installer/release work, i18n content (structure already in place).
