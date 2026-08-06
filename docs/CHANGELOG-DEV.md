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

---

## 2026-08-06 — Phase 1: first blood

**Built.** Two working tools on top of a quarantine service, in the build order
the brief specifies: quarantine → executor → shutdown timer → brain gates →
cleaner → drawer UI.

- **Quarantine** (`core/quarantine.rs`). Same-volume `rename` where possible,
  copy + byte-for-byte verify + remove when crossing volumes, with the original
  left untouched if verification fails. The ledger row is written *before* the
  move, so a crash mid-move still knows where the file belongs. Restore never
  overwrites — a collision gets a ` (restored)` suffix and reports it. Retention
  purge (7 days, configurable) plus a size cap that purges oldest-first, both
  refusing to touch anything outside the quarantine root.
- **Executor** (`core/executor.rs`). Approved recommendations → one quarantine
  batch → one `ExecReport`. Every item is re-validated at execute time: the
  recommendation's own invariants, then the protected-path check, then the lock
  check — none of them trusted from the analyze phase, because the review screen
  can sit open for an hour. A failure or a lock skips that item only. Every item
  writes a `tracing` line.
- **Shutdown timer** (`modules/shutdown_timer.rs`). Duration and clock-time
  modes, shutdown/restart/sleep, replace-on-schedule with confirmation, >24h
  refused, warnings at T-5 and T-1, persistence across app restarts (re-armed
  only if the target is still ahead). Tray icon with a countdown tooltip and a
  cancel item. `--dry-run` flag.
- **Brain v0** (`core/brain/`). Hard gates only: protected path → red, locked
  file → red, rule match → the rule's declared bucket, nothing matched → red
  with an honest reason. `protected_paths()` is compiled-in Rust covering
  `%WINDIR%`, both Program Files trees, `%ProgramData%`, the profile root
  (exact, not its subdirectories), drive roots, `System Volume Information`, and
  our own app and quarantine directories.
- **Rule packs** (`rules/`, `core/brain/rules.rs`). `windows-temp`,
  `recycle-bin`, `thumbnail-icon-cache`, plus `rule-pack.schema.json`. The
  parser is a trust boundary: `deny_unknown_fields`, every constraint checked,
  and failures name the pack and the dotted field path.
- **Cleaner v0** (`modules/cleaner.rs`). Walks only the roots the packs point
  at, tags findings with pack and rule, and skips anything touched in the last
  hour even under a Safe rule.
- **Scanner, settings, scan store, logging** (`core/`). jwalk-based walker that
  never follows links and is cancellable per entry; typed settings over the
  key/value table; last-8-scans in-memory store; daily-rotating audit log in
  `{app_data}/logs`.
- **UI.** Cleaner 3-step flow (scan → three-bucket review grouped by pack, every
  row showing its reason, green preselected and yellow not → confirm dialog
  stating the total and the quarantine promise). Timer route with a big
  countdown driven by `shutdown:tick`. Quarantine drawer in the shell with
  restore, per-batch item lists and purge-now. The dummy module is gone.

**Decisions taken.**

- **Every irreversible OS call sits behind a trait** (`core/platform/`):
  `PowerController`, `RecycleBin`, `LockProbe`, plus `EnvResolver` for `%VARS%`.
  The real implementations refuse to act when `cfg!(test)` is true, and the app
  installs the real power controller in exactly one place — `lib.rs::run()`,
  only when `--dry-run` is absent. Two tests assert the refusal directly.
- **`EnvResolver` is what makes fixture tests honest.** Pointing `%TEMP%` at a
  tempdir means the tests exercise the *shipped* rule packs without the shipped
  packs ever touching the developer's real temp folder.
- **The "only quarantine may delete" rule is enforced against the source.**
  `no_other_module_removes_user_data` walks the crate's own `src/` and fails the
  build if any file other than `core/quarantine.rs` calls `fs::remove_*`.
- **The recycle bin skips the path gates.** Running a lock probe against
  `shell:RecycleBinFolder` made a perfectly emptyable bin look permanently
  locked — a real bug the end-to-end test caught. `Brain::classify_finding`
  routes shell objects straight to the rule.
- **Purge deletes the ledger row**; the rolling log keeps the audit trail. No
  migration was needed, so the schema is still at version 1.
- **A malformed rule pack refuses to start the app**; a rule whose `%VAR%` is
  unset is reported in `ScanResult.skipped_rules` and shown in the UI. Broken is
  a bug; missing is a fact about this machine.
- **Registered modules must have a route.** A test asserts
  `src/routes/<id>/+page.svelte` exists for every module in the registry.
- **Rule packs ship as a bundled resource.** `tauri.conf.json` maps `../rules/`
  to `rules/` in the resource directory; without it an installed build would
  find no packs, because the dev fallback path (`CARGO_MANIFEST_DIR`) does not
  exist on a user's machine. The fallback stays for dev builds, so editing a
  pack and re-running needs no copy step.

**Verification.** `cargo clippy --all-targets -- -D warnings` clean,
`cargo fmt --check` clean, `cargo test` 225/225 green (218 unit + 7 integration),
`cargo build --release` OK, `npm run build` OK, `npm run check` 0 errors /
0 warnings.

**Deferred / not verified.**

- The window and tray were **not launched visually** — `tauri dev` blocks the
  agent session. The tray, the notifications and the countdown are evidenced by
  unit tests over the engine and a clean `svelte-check`, not by a screenshot.
  Someone should run `npm run tauri dev -- -- --dry-run` and watch a 2-minute
  timer through both warnings before this is called done.
- Notification **action buttons** are not implemented (Tauri v2 has no reliable
  Windows support); cancel is via the route and the tray. The capability file
  deliberately does **not** grant `notification:default` — notifications are
  raised from Rust, which the ACL does not gate, and handing the WebView the
  permission it does not need would be the wrong default. Windows toasts also
  want an installed shortcut for a stable AppUserModelID, so a `tauri dev` run
  may show them differently from an installed build.
- The cleaner never proposes a **directory**, so emptied temp folders keep their
  empty subdirectories. Quarantine handles directories already.
- **Yellow-bucket packs, scoring, heuristics and the LLM layer** are out of
  scope per the brief. The yellow bucket renders and stays empty; red is
  populated by the gates.
- `owner_process_absent` is parsed and validated but not acted on — it belongs
  with the browser packs.
- CI is still **unverified**: no git remote exists, so the workflow has never
  run. The workflow was not updated for the new `cargo fmt`/integration-test
  steps beyond what phase 0 already ran.
