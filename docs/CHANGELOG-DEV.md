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
  never run and the README badge points at an `OWNER/winknife` placeholder.
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
- The product name is still the `winknife` codename. `src/lib/branding.ts` holds
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

---

## 2026-08-06 — Phase 1: security remediation

**Built.** Nothing new. An adversarial security review blocked phase 1; this is
every finding it raised, fixed, with a regression test per fix. 262 unit tests
(was 218) and 11 integration tests (was 7).

### Critical

- **C1 — the protected-path gate was a string comparison.** Windows accepts
  several spellings of the same file that a lexical comparison reads as
  different places, so a community rule pack could name `C:\Windows\System32`
  as something the list had never seen, have it walked, quarantined and rated
  Safe. Six parts:
  - **(a)** `executor.rs` canonicalises through `dunce` before consulting the
    list, and refuses anything it cannot resolve. A missing path still falls
    through to the move, which reports the miss properly. New dependency:
    `dunce` (`std::fs::canonicalize` always returns `\\?\` paths, which is the
    very form the list cannot match).
  - **(b)** `ProtectedPaths` strips `\\?\` and `\\.\`, discards the trailing
    dots and spaces Win32 discards, and resolves `..` lexically — in list
    entries and probes alike. A path that still carries a `..` is reported as
    protected rather than guessed at: a lexical answer is the wrong one as soon
    as a junction is involved, and nothing legitimate reaches the gate with one.
  - **(c)** `parse_pack` rejects patterns containing `..`, a `\\?\` / `\\.\`
    prefix, or an 8.3 component, in `paths` and `exclude` alike. Malformed
    pack, app refuses to start, same as every other pack validation.
  - **(d)** The scanner resolves every walk root before walking it; a root that
    is, or resolves into, a protected location is skipped and named in
    `skipped_rules`. `CleanerContext` carries the gates for this. The *spelled*
    root is what gets walked — the globs were compiled against that spelling —
    and the *resolved* root is what decides whether to walk at all.
  - **(e)** `C:\Windows`, `C:\Program Files`, `C:\Program Files (x86)` and
    `C:\ProgramData` are compiled in unconditionally. Environment variables now
    only ever widen the list.
  - **(f)** Table-driven regressions over every probe form, plus pack-rejection
    cases for each banned shape. The 8.3 case is proved at the executor, with a
    fixture whose short name comes from `GetShortPathNameW`; the junction case
    likewise, skipped when the environment cannot create links.

### High

- **H1 — `cleanup_partial` was an unguarded `remove_dir_all`.** On the restore
  path its argument is a user directory, so a failed copy onto an existing
  folder deleted the folder. It now tracks whether the destination pre-existed
  and only removes what this call created; `take` also refuses any destination
  outside the quarantine root; failures log instead of `let _ =`.
- **H2 — restore could overwrite an existing file.** `Path::exists` answers "no"
  for a path we merely cannot see, and `fs::rename` is `MoveFileEx` *with*
  `MOVEFILE_REPLACE_EXISTING`. Occupancy is probed with `symlink_metadata` and
  anything but a clean not-found counts as occupied; the rename itself is
  `MoveFileExW` **without** the replace flag, so the suffix loop retries on
  `AlreadyExists` instead of clobbering.
- **H3 — timer cancel cleared memory before persistence.** A failed write
  resurrected the shutdown on the next start, after the user had watched the
  countdown disappear. Persistence is cleared first; the tray path retries and
  logs, and leaves the timer armed if it cannot clear it.
- **H4 — `shutdown_schedule` armed before persisting.** A failed write left a
  live countdown while the UI reported failure. `ShutdownTimer::prepare`
  resolves without arming, the command persists, then `arm` runs.
- **H5 — `cfg!(test)` guards are inert in integration-test binaries.**
  `cfg(test)` is set for the compilation unit being tested, so the guards inside
  `SystemPower` and `WindowsRecycleBin` were false in `tests/` — where both
  types were `pub`-reachable. Replaced by a process-global `AtomicBool` armed
  exactly once in `lib.rs::run()`; `arm_real_os_effects` is `pub(crate)`, so
  nothing outside the crate can arm it. Both types are crate-private and no
  longer re-exported; they are reachable only as the trait objects
  `platform::real_power_controller` / `real_recycle_bin` return.
  `tests/os_effects_stay_disarmed.rs` holds both, calls them, and asserts the
  refusal from the far side of the boundary.
- **H6 — the recycle bin emptied off a bare Uuid.** The one irreversible action
  in a batch now needs `confirm_empty_recycle_bin` on the `execute` command;
  without it the item is Skipped with a reason. It empties at most once per
  batch, and reports what it actually freed by querying the bin again
  afterwards rather than trusting the analyze-time figure.
- **H7 — retention policy unvalidated.** `RetentionPolicy::validated` clamps
  retention to [1, 365] days and the size cap to a 64 MB floor. A stored `0`
  used to mean "purge everything on the next start".

### Medium

- **M1** — `clippy.toml` disallows `fs::remove_file`, `fs::remove_dir`,
  `fs::remove_dir_all` and `fs::rename` **by resolved path**, which an aliased
  import or a fully-qualified call cannot dodge. Four `#[allow]`s, all in
  `quarantine.rs` and all commented. The grep test survives as a smoke check and
  now reads `tests/` and `build.rs` too.
- **M2** — `restore_item` marks the ledger row before the move and puts it back
  on failure, mirroring `take`.
- **M3** — the compensating DELETE after a failed move logs its own failure.
- **M4** — the size-cap loop stops at an item that frees nothing instead of
  dooming every remaining item to reclaim zero bytes; an unmeasurable directory
  is an error rather than zero bytes.
- **M5** — `PurgeReport::affected_batches` names the batches a purge took items
  out of.
- **M6** — a cross-volume copy keeps modification times and the read-only flag,
  and verification checks them, so the preservation is enforced rather than
  hoped for. `destroy` clears read-only before removing, or a preserved flag
  would make an item unpurgeable on Windows. The rename primitive is injectable,
  so a test can force the cross-volume branch.
- **M7** — `copy_dir_recursive` never follows a link, and a tree containing one
  is refused rather than moved. `dirs_are_identical` compares `symlink_metadata`
  file types.
- **M8** — the `requires_admin` guard was byte-indexed and exempted anything
  starting with `%` or `\\`. Now a character-based shape check at parse time
  (UNC and verbatim require admin) plus a profile-containment check after
  expansion.
- **M9** — the lock probe answers `Inaccessible` for a directory rather than
  `Free`, which the callers read as permission to move.

### Low

- **L1** — the cross-volume errno is 17 on Windows and 18 elsewhere; they were
  accepted interchangeably, with a dead clause.
- **L2** — timestamps are written `to_rfc3339_opts(Millis, use_z)`, so the text
  order the retention query compares on matches the time order. Still
  parse-compatible with rows written by the old format, so no migration.
- **L3** — `PersistedTimer` carries its `ScheduleMode`, and `rearm` re-resolves
  an `At` target. Only for a timer still in the future, so a stale one is still
  discarded rather than rolled on to tomorrow. Per-tick re-resolution is
  deliberately not done: a countdown that jumps mid-count is worse than one
  that is right at every restart.
- **L4** — unparsable ledger rows are skipped and counted in
  `QuarantineStats::unreadable_rows` rather than becoming nil-uuid items dated
  1970 that the drawer offered to restore.
- **L5** — walk roots are deduplicated by resolved identity, so `%TEMP%` and
  `%LOCALAPPDATA%\Temp` are one walk with both rules attached rather than two.

**Decisions taken.**

- **M7 refuses rather than skips.** The review allowed "skip links with a
  per-item Skipped report", but skipping and then removing the source destroys
  the link — a permanent loss of user data, which CLAUDE.md forbids outright.
  A directory containing a link is therefore refused for a cross-volume move,
  the original untouched, and the error names the links. Same-volume moves are
  unaffected (a rename carries links intact), and phase 1 never proposes a
  directory, so nothing reachable from the UI changes.
- **M5 reports rather than purges whole batches.** Purging whole batches would
  destroy items still inside their retention window because a sibling was old —
  more destructive than the cap requires. Item-level eviction stays; the report
  names the affected batches.
- **H3's tray path leaves the timer armed** when persistence will not clear
  after three attempts. A visible timer the user can cancel again beats an
  invisible one that comes back on the next start.
- **A `..`-carrying path is treated as protected**, not merely resolved. See
  C1(b).

**Wire-format changes** (mirrored in `src/lib/types.ts` in the same commits):
`QuarantineStats.unreadableRows`, `PurgeReport.affectedBatches`, and
`execute(scanId, approved, confirmEmptyRecycleBin)`.

**Verification.** `cargo fmt --all --check` clean, `cargo clippy --all-targets
-- -D warnings` clean, `cargo test` 273/273 green (262 unit + 11 integration),
`npm run check` 0 errors / 0 warnings, `npm run build` OK.

**Deferred / not verified.**

- The 8.3 and junction regressions **skip themselves** where the environment
  cannot provide the fixture (8.3 generation disabled, or link creation without
  developer mode). Both ran for real on the development machine; CI should be
  checked once a remote exists.
- `RetentionPolicy` is validated at the two constructors the app uses;
  `Quarantine::with_root` deliberately still takes a policy verbatim, because
  the size-cap tests need caps far below any sane floor.
- The **UI does not yet surface** `unreadableRows` or `affectedBatches`. Both
  are carried to the frontend and typed; showing them is a drawer change and
  was not in this brief's scope.
- Still not launched visually — same caveat as the phase-1 entry above.

## 2026-08-06 — Phase 1 security remediation, round 2

Round 1 closed 19 findings; the re-review still returned BLOCK. It found a 9th
protected-path bypass the round had missed, and one round-1 fix had introduced a
new HIGH. Everything below was reproduced against the real code with a probe
before being fixed, and re-probed afterwards.

### C2 — every gate missed `\localhost\C$\Windows\System32\ntoskrnl.exe`

The administrative share is the local disk under another name, it resolves from
a **non-elevated** process, and canonicalising it yields a third spelling
(`\?\UNC\localhost\c$\...`) that the list did not recognise either. Proven end
to end before the fix: a pack with pattern `\localhost\C$\Windows\Temp\**` was
ACCEPTED, the root was WALKED, and the file came back `bucket=Safe
action=Quarantine`. `\127.0.0.1\C$\...` behaved identically.

**Two rounds of rejecting bad spellings by name both shipped a bypass, so that
approach is now closed.** The gate is an allow-list:

- `brain::protected::gate_path(path) -> GateVerdict` is the single chokepoint.
  It canonicalises and accepts **only** a plain local drive-letter path — after
  an optional `\?\` prefix the first component must be `X:` followed by a
  separator. Everything else is `Refused`, and refused is not allowed: the
  caller skips the item and reports why. The shape is checked on the raw input
  *and* on the canonical form, because a local path can resolve onto a share.
- Both callers route through it and neither may fall back to the raw string:
  `modules::cleaner::resolve_root` (walk roots) and `core::executor::apply_one`
  (execute time). `executor`'s private `canonical_gate_path`/`GatePath` are gone.
- `rules::unsafe_pattern_shape` now rejects any pattern starting with `\` at
  all, plus `\??\`, on top of the existing `..`/verbatim/8.3 rejections.
- `strip_windows_prefix` also strips `\??\`, and `ProtectedPaths::reason_for`
  now reports a non-drive-anchored path as protected-because-unjudgeable rather
  than "not protected" — the lexical list stays as defence in depth, but it is
  no longer the only thing between a rule pack and `C:\Windows`.

### H8 — a zero-byte item permanently disabled the quarantine size cap

Round 1's fix for M4 used `break`. Candidates are ordered `quarantined_at ASC`,
so one empty `.tmp` at the head aborted eviction with nothing doomed, and a
quarantine over its cap never shrank for the whole retention window. Now
`continue`: a zero-byte item cannot reduce `over_by`, so it is skipped and real
items keep being evicted. The loop consumes a finite snapshot, so it terminates.

### M10 — `cleanup_partial` could still delete a user file

`destination_existed` was sampled *before* the copy. A file appearing at `to`
between probe and `create_new` made the copy fail `AlreadyExists` with the
probe still saying "did not exist", so cleanup deleted it — and on the restore
path `to` is a user path. `copy_file` and `copy_dir_recursive` now report
whether **they** created `to`, and cleanup keys on that.

### M11 — three safety tests self-skipped silently

`scanner::symlinks_are_never_followed`,
`executor::a_link_pointing_into_a_protected_directory_is_refused` and
`cleaner::a_walk_root_that_resolves_into_a_protected_location_is_refused`
asserted nothing on an unprivileged machine while the suite stayed green. The
audit found **two more** the review had not named
(`quarantine::symlinks_are_never_quarantined`,
`quarantine::a_directory_containing_a_link_is_never_moved_across_volumes`) and
one legitimately environment-dependent skip
(`executor::an_8_3_alias_of_a_protected_directory_is_refused`). All six now go
through `core::test_support::privileged_operation_unavailable`, which **fails**
unless `WINKNIFE_ALLOW_SKIP_LINK_TESTS=1` is set, and announces the skip on the
process's real stderr so libtest's per-test capture cannot swallow it. CI does
not set the variable and has a step that fails if anything else does. Three new
privilege-free companions assert the same logic on every machine.

### M12 — an in-crate test could arm the process-global OS-effects flag

`arm_real_os_effects` was `pub(crate)`, so a `#[cfg(test)]` module could arm it
and unlock the real `SystemPower`/`WindowsRecycleBin` for every later test in
that binary. It is now `#[cfg(not(test))]` — a test that calls it does not
compile — and the module header states exactly what is and is not guaranteed
instead of overclaiming.

### L6, L7

- The trailing-dot/space comment was factually wrong. Ground truth:
  `C:\Windows.\System32\ntoskrnl.exe` resolves (the dot is discarded),
  `C:\Windows \System32\...` is NotFound. Trimming spaces is a harmless
  over-refusal; the justification now says so.
- `clippy.toml` disallows `std::fs::File::create` and
  `std::fs::OpenOptions::truncate`. Both were verified to fire by compiling a
  deliberate violation; no production code needed an `#[allow]`.

**Verification.** Verdicts were read out of a compiled probe against the real
`gate_path` / `resolve_root` / `parse_pack` / `execute`, not off the source.
After the fix every attack spelling is `REFUSED` by the gate *and* `protected`
by the list; the hostile pack is rejected at parse; the walk root is refused;
`analyze` returns `DoNotDelete`/`InfoOnly`/`savings=0`; and the executor skips
with `reclaimed=0`. `cargo fmt --all --check`, `cargo clippy --all-targets --
-D warnings`, `cargo test` (283 green), `npm run check` (0 errors), `npm run
build` all pass.

**Deferred.** `quarantine::symlinks_are_never_quarantined` has no
privilege-free companion — `take`'s refusal reads `symlink_metadata` directly,
and mocking it means injecting a metadata probe into `Quarantine`, which is a
design change this brief did not ask for. The copy layer's link handling *is*
covered without privileges.

## 2026-08-06 — Rename: cleanify → WinKnife

**Built.** Nothing new — a mechanical, repository-wide rename. No logic or
behaviour changed.

- The project was developed under the codename `cleanify` through phase 1.
  This commit renames every occurrence to **WinKnife**: the Cargo package and
  lib target (`winknife`, `winknife_lib`, and every `winknife_lib::` import in
  `src-tauri/tests/*.rs`), the Tauri `productName`/`identifier`
  (`com.winknife.app`), the database file (`winknife.sqlite3`), the log prefix
  and filter env var (`winknife.log`, `WINKNIFE_LOG`), the tray id, the CI
  artifact names (`winknife-portable`, `winknife-installer`), the `APP_NAME`
  UI constant, and every remaining prose mention across the docs.
- **The Tauri identifier change moves the application data directory.**
  `app_data_dir()` is keyed off `identifier`, so `com.cleanify.app` →
  `com.winknife.app` orphans any quarantine database or log a local dev build
  already wrote under the old path. No released version of the app ever
  existed, so no user data is affected.
- The GitHub repo now exists at `armadavalor/WinKnife` (public, MIT). The CI
  badge, `Cargo.toml`'s `repository`, and the rule-pack schema's `$id` point at
  it instead of the `OWNER/cleanify` placeholder; `LICENSE` reads "WinKnife
  contributors". The README's rename checklist is removed — it described
  exactly this change and is now stale by definition.

**Verification.** `cargo fmt --all --check` clean, `cargo clippy --all-targets
-- -D warnings` clean, `cargo test` 283/283 green (272 unit + 8 + 3
integration — the same count as before the rename), `npm run check` 0 errors /
0 warnings, `npm run build` OK.

**Deferred / not verified.**

- As in every prior phase, this development machine has no Developer Mode, so
  the five link-dependent safety tests ran under
  `WINKNIFE_ALLOW_SKIP_LINK_TESTS=1` rather than for real; CI, which never sets
  that variable, is where they actually execute.
- `npm run tauri build` (the installer bundle) was not run; only `cargo build`
  (debug) built the renamed binary, confirmed at
  `src-tauri/target/debug/winknife.exe`.
- CI itself is still unverified — the workflow has not run against the new
  remote yet.

## 2026-08-06 — First CI run: the protected list's own spelling

**Built.** The first CI run on `windows-latest` was red: 270 passed, 2 failed —
and the two failures were precisely the symlink-dependent safety tests that had
never actually executed on the development machine (no Developer Mode). The M11
change that turned a silent skip into a failure did its job on the first try:
the failures were a real product bug, not a test artefact.

**The bug.** `ProtectedPaths` stored entries in whatever spelling it was handed,
while `gate_path` canonicalises the path it is asked about. On the GitHub runner
`%TEMP%` is `C:\Users\RUNNER~1\AppData\Local\Temp` — the 8.3 short form — so the
fixtures built their protected entries from a short-name path and the gate came
back with the long one (`C:\Users\runneradmin\...`). Two spellings of one
directory, compared component by component, do not match, so the list answered
"not protected" and the walk root was walked / the file was quarantined.

This is not a CI quirk. `%TEMP%` genuinely is `C:\Users\USERNA~1\AppData\Local\
Temp` on a great many real accounts (the development machine's is), and
`%WINDIR%`, `%USERPROFILE%` and `%ProgramFiles%` can reach us in 8.3 form too.
The real app builds its protected list from exactly those variables, so on those
machines a protected location silently stopped being protected. Same class as
the two CRITICALs fixed in the last round: comparing two different spellings of
one path.

- **Fix** (`core/brain/protected.rs`, new `widen`, called by
  `ProtectedPaths::new` and `::push`, so `from_env` and `from_trees` both get
  it). Every entry is registered in both spellings: the normalised text it was
  written in, plus its canonical form when the filesystem spells that same
  directory differently.
  - Both, not just the canonical one: the gate asks in the canonical spelling,
    but `reason_for` is also the defence-in-depth layer for callers that only
    have text — and that text may well be the short form the variable was in.
    Adding a spelling only ever protects more.
  - Nothing is ever dropped. A directory this machine does not have cannot be
    canonicalised, and losing the entry would turn a missing folder into an
    unprotected one; the hardcoded floor (`C:\Windows`, `C:\Program Files`,
    `C:\Program Files (x86)`, `C:\ProgramData`) therefore survives on machines
    that have those directories and on machines that do not.
  - Only a plain local drive-letter path is canonicalised, reusing
    `plain_local_shape` as the guard. A drive-root entry is `C:`, whose
    canonical form is that drive's *current directory* — a real path and the
    wrong one to protect exactly — and the volume-metadata entry is a bare name
    belonging to no drive at all.
  - `reason_for`, `gate_path` and `plain_local_shape` are untouched: the
    allow-list gate keeps its shape and every attack spelling still returns
    REFUSED.
- **Regression tests** (`core/brain/protected.rs`). Three, all privilege-free:
  an entry registered as an 8.3 alias must match the canonical probe;
  `from_env` fed 8.3-form `%WINDIR%`/`%USERPROFILE%` must still protect the real
  directories (and must not widen the profile root's *exact* scope into a tree);
  and widening must not invent an entry out of a drive root or the
  volume-metadata name. `short_path_name` (`GetShortPathNameW`) moved from the
  executor's test module to `core/test_support.rs` so all three files share it.
- **CI** (`.github/workflows/ci.yml`). `actions/checkout` and
  `actions/setup-node` bumped `@v4` → `@v5`; both v4 majors run on the
  deprecated Node 20 and the first run warned about it.

**Verification.** The diagnosis was proven by execution, not by reading. Before
the fix, a list built from a real 8.3 path missed the canonical probe:
registered `C:\Users\ARMADA~1\...\PROTEC~1`, probed
`C:\Users\ARMADA-L5-PRO\...\Protected Program Files\kernel.sys` → not protected.
Because the two CI failures need a reparse point and this machine cannot create
symlinks, both were then reproduced locally with a *junction* (`mklink /J`,
which needs no privilege and which `canonicalize` resolves the same way): with
the widening disabled the executor reported `Succeeded` — it quarantined the
protected file — and `resolve_root` returned `Walk`, byte-for-byte the two CI
failures; with it enabled both refuse, the executor with "protected location,
not cleanable" and the walk root with "resolves to". The junction probes were
temporary and are not committed.

`cargo fmt --all -- --check` clean, `cargo clippy --all-targets -- -D warnings`
clean, `cargo test` 286/286 green (275 unit + 8 + 3 integration), `npm run
check` 0 errors / 0 warnings, `npm run build` OK.

**Deferred / not verified.**

- The five link-dependent safety tests again ran under
  `WINKNIFE_ALLOW_SKIP_LINK_TESTS=1` (no Developer Mode here):
  `a_link_pointing_into_a_protected_directory_is_refused`,
  `a_walk_root_that_resolves_into_a_protected_location_is_refused`,
  `symlinks_are_never_followed`, `symlinks_are_never_quarantined`,
  `a_directory_containing_a_link_is_never_moved_across_volumes`. The first two
  are the ones CI failed, and the junction probes above stand in for them
  locally. The 8.3-alias test (`an_8_3_alias_of_a_protected_directory_is_refused`)
  did *not* skip — this volume mints aliases — so the new regression tests
  really ran.
- `actions/upload-artifact@v4` is on Node 20 as well and will warn on the first
  run that reaches it; left alone because those steps never ran in the failing
  build and the brief named only the two.
- `npm run tauri build` was not run.

## 2026-08-06 — Round 3: the app did not start, and why that kept happening

**What was built.** A third adversarial review returned BLOCK, and the headline
was not a security hole: **WinKnife did not boot** on this developer machine or
on the GitHub runner.

`compile_spec` (`brain/rules.rs`) asks whether a rule's expanded walk root sits
inside `%USERPROFILE%`, and refuses to load the pack when it does not. Windows
hands `%TEMP%` over in 8.3 form on any account whose name does not fit 8.3 —
`C:\Users\ARMADA~1\AppData\Local\Temp` here, `C:\Users\RUNNER~1\...` on CI —
while `%USERPROFILE%` arrives in full. `is_inside` compared the two as component
text, judged the temp folder to be outside the profile, and refused every
shipped pack. `lib.rs` treats a rules-load failure as fatal by design and
propagates it out of Tauri's `setup`, so the window never opened. It failed
closed, so no data was ever at risk — but it was unshippable.

The instruction was explicit that `requires_admin: true` on the temp rules is
*not* the fix: that flag silences the profile-containment guard, which is a real
safety check.

**The structural fix.** Three rounds had each fixed one instance of one defect:
**two spellings of one path compared as raw text.** `widen` treated the symptom
in `ProtectedPaths`; `rules.rs` and `brain/mod.rs` still had it. So the fix this
round is a module, not a patch.

- **New: `core/path_compare.rs`.** The one place in the crate where two paths
  are compared. `CanonicalPath::resolved` canonicalises through
  `dunce::canonicalize`; `same_path`, `is_inside` and `has_traversal` are built
  on it. A path that is not on disk cannot be canonicalised, so the longest
  ancestor that *does* exist is canonicalised and the missing tail re-attached —
  a tail that does not exist cannot hold a reparse point, so nothing is guessed
  at, and `C:\` exists everywhere, so this degrades gently rather than falling
  back to pure text. `CanonicalPath::lexical` is the text-only form, kept for
  the one loop that cannot afford a syscall per entry.
- The Windows path text semantics (verbatim/device/NT prefixes, trailing dot and
  space, lexical `..`, the UNC share root that is one component) now live there
  once. `protected.rs` delegates to them instead of holding its own copy.

**Findings fixed.**

- **C3 (CRITICAL, the boot failure)** — `rules.rs`. `compile_spec` now asks
  `path_compare::is_inside`, so a short-form `%TEMP%` is correctly seen to be
  inside a long-form `%USERPROFILE%`. `rules::is_inside` deleted.
- **H9 (HIGH)** — `brain/mod.rs`. `classify` consulted only the lexical list
  while the executor also canonicalised, so analyze and execute could disagree
  about one path: `entry=LONG probe=SHORT` was rated green while the delete was
  (correctly) refused. CLAUDE.md's hard rule has two halves and only one was
  enforced. New `Brain::path_refusal` is gate 1 in full — lexical list, then
  `gate_path`, then the list again on the canonical form — and it is now the
  *single* implementation both `classify` and `core::executor` call.
- **M13 (MEDIUM)** — `quarantine.rs::destroy`, the one function allowed to
  delete permanently. Its root guard was `Path::starts_with`, which counts `..`
  as an ordinary component, so `...\quarantine\..\..\Windows\System32` passed
  it. A `Component::ParentDir` is now refused outright before the containment
  check, and the containment check goes through `path_compare`. `take`'s
  destination guard got the same treatment.
- **L8 (LOW)** — `quarantine.rs`. A `create_new` `AlreadyExists` on the
  cross-volume branch was flattened into `MoveFailure::Failed`, so `move_out`'s
  retry loop never fired there and a restore onto a contested name errored where
  ` (restored)` would have worked. `copy_verify_remove_inner` now returns
  `MoveFailure` and reports `Occupied` as `Occupied`.
- **L9 (LOW)** — `platform/guard.rs`. The `compile_fail` doctest proving
  `arm_real_os_effects` is absent from a test build sat inside `#[cfg(test)]`,
  and rustdoc does not collect doctests from `cfg(test)` code — `Doc-tests
  winknife_lib` said `running 0 tests` on every green run. The claim is true;
  the evidence was theatre. Doctest dropped, replaced by a written statement of
  what actually enforces it (name resolution under `cfg(not(test))`,
  `pub(crate)` for other crates, `clippy --all-targets` compiling the test
  targets).

**The process gap.** Every existing test built a `MapEnv` of fabricated values,
so nothing had ever loaded the shipped packs against the environment the app
really starts in — which is exactly where it did not start.
`the_shipped_packs_load_against_the_real_process_environment` (`lib.rs`) now
loads `rules/*.json` through `SystemEnv` and asserts the brain builds, i.e. that
the app can boot on the machine running the test. Two companion tests pin the
mixed-spelling shape on real directories (`lib.rs`, `rules.rs`).

**Decisions taken.**

- `ProtectedPaths::reason_for` deliberately stays a lexical comparison, and says
  so in its doc comment. It runs one probe against every entry for every file
  examined; resolving per entry would be a syscall per entry per file. Both
  sides are resolved once elsewhere instead — `widen` resolves each entry when
  the list is built, `path_refusal` resolves the probe before asking.
- Glob matching (`CompiledRule::matches_path`) and the shape checks
  (`unsafe_pattern_shape`, `plain_local_shape`) stay raw text, with a comment at
  each explaining why: globs were compiled against the pack's own spelling and
  would stop matching their own directory if the candidate were rewritten, and
  "how is this written?" is a question about text that must be answered *before*
  anything is resolved.
- `widen` keeps its `plain_local_shape` guard. Resolving a bare drive (`C:`) or
  the bare volume-metadata name would ask the disk about somewhere else
  entirely.

**Verification.** The boot test was written *before* any fix and run against the
literal pre-fix tree, then re-run after — and the pre-fix run was repeated by
stashing the whole change set, to be sure. Before: `*** LOAD FAILED *** rule
pack windows-temp is invalid at rules.user-temp-files.requires_admin:
%TEMP%\** expands to C:\Users\ARMADA~1\AppData\Local\Temp, which is outside the
user profile C:\Users\ARMADA-L5-PRO`. After: passes. The nine attack spellings
were re-probed against the final tree and are REFUSED at all four surfaces
(`gate_path`, the lexical list, `path_refusal`, and the `classify` rating) —
`every_attack_spelling_is_refused_by_every_surface` prints the table under
`--nocapture`.

`cargo fmt --all --check` clean, `cargo clippy --all-targets -- -D warnings`
clean, `cargo build --release` OK, `cargo test` 302/302 green (291 unit + 8 + 3
integration; 16 new), `npm run check` 0 errors / 0 warnings, `npm run build` OK.

**Deferred / not verified.**

- The same five link-dependent safety tests ran under
  `WINKNIFE_ALLOW_SKIP_LINK_TESTS=1` (still no Developer Mode here):
  `a_link_pointing_into_a_protected_directory_is_refused`,
  `a_walk_root_that_resolves_into_a_protected_location_is_refused`,
  `symlinks_are_never_followed`, `symlinks_are_never_quarantined`,
  `a_directory_containing_a_link_is_never_moved_across_volumes`. Every
  8.3-alias-dependent test ran for real, including the four new ones, so the
  two-spellings machinery really was exercised.
- `npm run tauri build` was not run.
- The ancestor-walk fallback in `CanonicalPath::resolved` degrades to a pure
  lexical comparison only when nothing on the way up resolves — reachable in
  practice only for a drive this machine does not have. Left as is: it errs
  toward over-refusal, which is the direction to be wrong in.

---

## 2026-08-06 — Phase 2: brain + duplicates

**Built.** The scoring pipeline `score.base` was waiting for, the full rule-pack
set, the duplicate finder, and the settings page that turns pack toggles from a
JSON field into a user decision.

- **Brain v1 scoring** (`core/brain/score.rs`, `core/brain/mod.rs`).
  ARCHITECTURE section 4 end to end: hard gates, the rule's base score, heuristic
  modifiers, thresholds >=85 / >=40, and `max_bucket` as a ceiling a rule can
  never earn its way through. `score.rs` is pure, so the whole table of outcomes
  is a unit test. Modifiers: idle age (both directions), size for review
  candidates only, a known cache directory, and the owning process being open —
  which is the one modifier that lowers a *ceiling* rather than a score.
  `the_scoring_table` has 32 cases and prints what it asserts;
  `changing_a_rules_base_score_moves_its_bucket_predictably` varies one field and
  pins the walk down through the thresholds.
- **The age signal is read from two clocks.** `SystemLockProbe` opens every
  candidate, so on a volume with last-access updates enabled the first scan bumps
  the access time of everything it examined and a second scan would find nothing
  safe — every file would look freshly used, by us. Age *bonuses* need both
  clocks old; the freshness *penalty* is keyed on writes, which only the owning
  program makes.
- **`owner_process_absent` works** (`core/platform/process.rs`). Parsed and
  validated since phase 1 and acted on by nobody. The brain asks the process list
  itself rather than trusting a module to remember, and `CachedProcessProbe`
  keeps that to one snapshot per program name instead of one per finding — a
  `%LOCALAPPDATA%` browser cache is tens of thousands of files.
- **Six new rule packs.** `browser-cache` (Chrome, Edge, Brave, Firefox; green
  closed, yellow open), `crash-dumps-logs`, `installer-leftovers`,
  `old-downloads` (capped at needs_review by its ceiling, proven exhaustively),
  `dev-caches` (off by default per module spec 02) and
  `windows-update-leftovers` (informational only — see the decision below). One
  fixture entry per pack in the cleaner's `Fixture`, and the manifest test now
  asserts the pack that claimed each file as well as its bucket.
- **Pack toggles** (`CleanerSettings`, `core/settings.rs`). Only *decisions* are
  stored, under `module.cleaner`, so a newly shipped pack keeps its own
  `enabled_by_default` instead of inheriting somebody's stale database. A pack
  that is off is not walked at all, and does not appear in `skipped_rules` —
  "you asked for this not to run" is not a partial scan.
- **Duplicates** (`modules/duplicates.rs`). Staged size -> partial hash
  (first+last 4 KB, blake3) -> full blake3 with rayon, progress and cancellation.
  Hard links and junctions collapse by volume-serial + file-index identity
  (`core/platform/file_id.rs`) and are *reported*, because a user who made that
  link deliberately deserves to be told why their obvious duplicate is missing.
  The keeper heuristic scores the four signals module spec 03 names and the UI
  lets a person override it per set or in bulk. Every removal is NeedsReview and
  no higher.
- **Cancellation is reachable** (`core::state::CancelRegistry`, `cancel_scan`).
  ARCHITECTURE section 3 listed the command; phase 1 shipped a token nothing
  could trip. A full-hash pass over a Pictures folder is minutes of work, so the
  duplicates route has a stop button that does something.
- **UI.** Collapsible per-pack groups in the cleaner's review (collapsed is the
  opt-in state, so a scan opens showing everything), the duplicates route with
  sets sorted by wasted bytes, keeper badges, per-set and bulk keeper switches,
  and extension/size filters, and a settings page rendered from the packs that
  actually loaded — a community pack dropped into `rules/` appears with no UI
  change.
- **Junction companions for the five symlink safety tests**
  (`core/test_support.rs::make_junction`, architect-approved). `mklink /J` needs
  no privilege and `canonicalize` resolves a junction identically, so "the walker
  never follows a link", "a link into a protected directory is refused", "a walk
  root that resolves into one is refused", "a link is never quarantined" and "a
  directory containing a link is never moved across volumes" now assert on every
  developer machine instead of only in CI.

**Decisions.**

- `windows-update-leftovers` cannot do what module spec 02 asks. Its only
  location is `%WINDIR%\SoftwareDistribution\Download`, `%WINDIR%` is protected
  in code, and CLAUDE.md says a protected path never receives a Safe rating *or*
  a delete action. Shipped as `info_only` + `do_not_delete` and off by default,
  so it can report and never act and does not put a permanent refusal line in
  front of every user. `no_shipped_rule_targets_a_protected_path` was
  **tightened** rather than relaxed: a rule naming a protected path must be
  structurally unable to propose anything, and the brain is asked to prove it
  refuses a file under it. Escalated in `docs/DECISIONS-NEEDED.md`.
- Duplicate confidence is a flat 75 rather than a `brain::score` number. A blake3
  match is a fact about bytes, not a heuristic; the uncertainty is entirely about
  which copy a person wants, which is why the bucket is NeedsReview.
- The i18n string for `skipped_rules` no longer claims the cause is an unset
  folder. It carries protected-root refusals too, and it did before this phase.

**Deferred.** `QuarantineStats.unreadableRows` and `PurgeReport.affectedBatches`
stay unsurfaced (architect deferred both to phase 3). No LLM layer, no treemap,
no startup manager. `%WINDIR%\*.log` and `%WINDIR%\Downloaded Program Files` from
module spec 02 are not shipped as rules at all, for the same protected-tree
reason as the pack above.
