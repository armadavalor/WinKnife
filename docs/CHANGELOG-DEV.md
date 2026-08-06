# Development changelog

Append-only log of completed tasks: date, what was built, notable decisions,
anything deferred. Newest entries at the bottom.

---

## 2026-08-06 — Phase 0: skeleton

**Built.** A Tauri v2 + Svelte 5 + TypeScript + Tailwind v4 app that compiles,
with the module architecture in place and zero features.

- **Scaffold & hygiene.** `create-tauri-app` (template `svelte-ts`, SvelteKit +
  `adapter-static` in SPA mode) generated into a temp directory and moved in
  next to the existing docs. MIT `LICENSE`, README stub with CI/licence badges
  plus dev-setup and rename checklist, extended `.gitignore`, this changelog and
  `docs/DECISIONS-NEEDED.md`. Repo initialised locally on `main`; nothing is
  pushed anywhere.
- **Module contract** (`core/module.rs`). `ToolModule` with
  `id/manifest/scan/analyze/execute`, `ModuleManifest`, `ModuleCapabilities`,
  `ScanContext`, `ExecContext`, and `ModuleRegistry` which preserves
  registration order (the sidebar order) and rejects duplicate ids.
  `scan/analyze/execute` have no-op defaults so a non-file module (the shutdown
  timer) only implements `id` and `manifest`. `DummyModule` proves the pattern.
- **Shared data model** (`core/types.rs` + `src/lib/types.ts`). `Finding`,
  `FindingKind`, `RiskBucket`, `Action`, `Recommendation`, `ExecOutcome`,
  `ExecItemResult`, `ExecReport`, all serde camelCase with wire-format tests
  pinning the exact JSON the TypeScript mirror describes.
- **Storage** (`core/db.rs` + `migrations/001_init.sql`). rusqlite opened in the
  app data dir, migration runner keyed on `pragma user_version`, migration 001
  creating `scans`, `actions`, `quarantine`, `settings`.
- **Events** (`core/events.rs`). Typed payloads and emit helpers for
  `scan:progress`, `scan:done`, `exec:progress`, `quarantine:changed`, plus a
  `ProgressThrottle` capping progress at 10 events/sec.
- **IPC** (`commands.rs`, `lib.rs`). Registry and migrated connection registered
  as Tauri managed state at startup; `list_modules` works end to end.
- **UI shell.** Sidebar and dashboard cards both rendered from `list_modules`;
  dark theme with all colour tokens in `src/app.css`; `t()` i18n helper wrapping
  every user-facing string; `lib/api.ts` the only file calling `invoke`;
  registry held in a Svelte 5 runes store. `/` redirects to `/dashboard`.
- **CI.** `.github/workflows/ci.yml` on `windows-latest`: rustfmt, clippy
  `-D warnings`, `cargo test`, `svelte-check`, UI build, `tauri build`, with the
  portable exe and NSIS installer uploaded as artifacts.

**Decisions taken.**

- **`run()` stays in `lib.rs`, not `main.rs`.** ARCHITECTURE section 1 puts the
  builder in `main.rs`, but the Tauri v2 layout needs it in the lib target for
  the mobile entry point and so unit tests can see the crate. `main.rs` is a
  one-line shim.
- **`Brain` is an empty placeholder.** The brain is out of scope for phase 0,
  but `analyze(&self, findings, brain)` is the signature the architecture
  specifies. Keeping it verbatim with a zero-logic `Brain` means phase 1 fills
  in a body instead of changing every module.
- **Invariants live in code, not in docs.** `Recommendation::new/validate`
  rejects a blank reason, confidence above 100, an empty finding list, and any
  mutating action on a `DoNotDelete` item. `Action` has no raw `Delete` variant.
- **A new module is literally 1 registry line.** `modules/mod.rs` uses a
  `register_modules!` macro that emits both the `mod` declaration and the
  registry entry, so the phase acceptance claim is exact rather than
  approximate.
- **SQLite tables are `STRICT`, `foreign_keys` is on per connection, journal
  mode is WAL.** This app deletes things; the quarantine ledger has to be the
  most trustworthy part of it.
- **Module display names are translated by convention.** The UI looks up
  `module.<id>.name` / `.description` and falls back to the English string the
  Rust manifest supplied, so a Turkish dictionary stays a drop-in.

**Verification.** `cargo clippy -- -D warnings` clean (also with
`--all-targets`), `cargo fmt --check` clean, `cargo test` 49/49 green,
`cargo build --release` OK (10.0 MB exe), `npm run build` OK, `npm run check`
0 errors / 0 warnings. The "second module = 2 files + 1 registry line"
acceptance test was performed: `dummy2.rs` + `routes/dummy2/+page.svelte` + one
line, diffstat `3 files changed, 58 insertions(+)`, registry then reported
`["dummy", "dummy2"]`; the experiment was reverted.

**Deferred / not verified.**

- CI is **unverified** — the project has no git remote yet, so the workflow has
  never run and the README badge points at an `OWNER/cleanify` placeholder.
  `npm run tauri build` (installer bundling) was likewise not executed locally;
  only `cargo build --release` was.
- The app window was not launched visually (`tauri dev` blocks the agent
  session). "Sidebar shows the dummy module" is evidenced by the registry tests,
  the `list_modules` delegation test and a clean `svelte-check`, not by a
  screenshot.
- `tracing` and the rolling audit log (ARCHITECTURE section 8) are not wired up
  — they belong with the executor in phase 1, which is what actually needs an
  audit trail.
- No `scanner.rs`, `executor.rs`, `quarantine.rs`, brain rules or `rules/*.json`
  yet; all phase 1+.
- The product name is still the `cleanify` codename. `src/lib/branding.ts` holds
  the single UI-side constant, and the README lists every other place it
  appears.
