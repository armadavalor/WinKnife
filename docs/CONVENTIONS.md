<!-- durum: aktif -->
# Conventions

## Rust
- Edition 2021+, `rustfmt` defaults, `clippy -- -D warnings` clean.
- No `unwrap()/expect()` outside tests and truly-infallible spots (comment why). Domain errors via `thiserror`.
- Filesystem/registry access only in `core/` and `modules/` — never in `commands.rs`.
- Windows-specific calls via `windows` crate behind small wrapper fns (mockable in tests).
- Public items get doc comments; module file starts with a short "what/why" header comment.

## TypeScript / Svelte
- Strict mode on. No `any`.
- `invoke()` only inside `src/lib/api.ts`; components call typed wrappers.
- Rust ↔ TS types mirrored by hand in `src/lib/types.ts` — when you change a serde struct, update it in the same commit.
- Svelte 5 runes; state in `lib/stores/`, components presentational where possible.
- Tailwind; shared UI primitives in `lib/components/`, don't restyle per-route.

## Naming
- Module ids: kebab-case (`shutdown-timer`). Events: `scope:verb` (`scan:progress`). Commands: snake_case verbs (`start_scan`).

## Commits & changelog
- Conventional Commits (`feat(cleaner): ...`, `fix(quarantine): ...`). One logical change per commit.
- Append to `docs/CHANGELOG-DEV.md` per completed task: date, what, notable decisions, deferrals.

## Tests
- New scoring rule or heuristic ⇒ table-driven test case in the same PR/commit.
- Anything touching quarantine/executor ⇒ tempdir round-trip test.
- Fixtures for scans live in `tests/fixtures/`; never scan the real machine in tests.

## Docs upkeep
- Scope questions/contradictions → `docs/DECISIONS-NEEDED.md` (create if absent), format: `- [ ] YYYY-MM-DD (phase N): question — safest interpretation taken`.
- User-facing strings in English for V1; wrap in a `t()` helper from day one so i18n (Turkish next) is a dictionary drop-in, not a refactor.
