<!-- durum: aktif -->
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
registered `C:\Users\USERNA~1\...\PROTEC~1`, probed
`C:\Users\USERNAME\...\Protected Program Files\kernel.sys` → not protected.
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
`C:\Users\USERNA~1\AppData\Local\Temp` here, `C:\Users\RUNNER~1\...` on CI —
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
%TEMP%\** expands to C:\Users\USERNA~1\AppData\Local\Temp, which is outside the
user profile C:\Users\USERNAME`. After: passes. The nine attack spellings
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

## 2026-08-06 — phase 2 review remediation

Adversarial review of phase 2 blocked on one HIGH finding plus five smaller
ones. The scoring pipeline and duplicates correctness were verified sound and
are untouched.

**H10 (the block) — the executor re-validated the gates but never the
justification.** `apply_one` re-checked `path_refusal` and `lock_state`, both of
which ask "is this still allowed?". Nothing asked "is this still *warranted*?" —
`owner_process`, `min_age_hours`, `match`, `content_hash` and `full_hash`
appeared nowhere in `executor.rs`. Analyze runs, the user reviews for as long as
they like, execute acts on reasoning that may have expired. Two consequences,
both reproduced: a duplicate edited into the newest unique file on the machine
was still moved out under "Byte-for-byte identical to `<keeper>`"; and a browser
cache scanned with Chrome closed was quarantined after Chrome was opened, because
`owner_process_absent` governed the rating and not the action.

Gate 4 now re-derives what the finding recorded and skips-and-reports on
mismatch, never failing the batch. Duplicates: full re-hash against the stored
hash, plus a check that the keeper still exists and is not another spelling of
this same file — if the keeper vanished, quarantining the remaining copy would
leave the user with none. Rules: the process probe re-asked **uncached** (the 2 s
memo answers the question the scan asked), and `min_age_hours` / `min_size_bytes`
/ `extensions` re-tested against fresh metadata. `full_hash` moved to
`core::content_hash` so the scan and the re-check compute the same number and the
executor does not reach up into a module.

**A1 — the scan-time lock probe mutated the filesystem.** `SystemLockProbe`
opened every candidate, which moves the last-access time on a volume where
updates are enabled — and that timestamp is a signal `brain::score` reads back,
so the scan mutated what it was measuring and the second scan of the day would
find nothing safe. Fixed at the cause with Windows' documented suppression
(`SetFileTime` with a last-access `FILETIME` of `{0xFFFFFFFF, 0xFFFFFFFF}`),
falling back to a plain read-only open where the extra access right is refused.
The two-clock reading stays as defence in depth.

**M14 — walk-root folding silently disabled a rule.** `plan_walks` folds roots on
canonical identity but kept the first rule's spelling, while `matches_path` is a
text glob anchored to each rule's own spelling — so two rules sharing a canonical
root spelled differently left every rule but the first unable to claim anything,
with nothing reported. The fold is kept (walking twice is what L5 fixed) and the
walked path is re-spelled into the absorbed rule's anchor. `plan_walks` also now
reports any rule that reached no group at all.

**M15** `copiesInSet` reported `Vec::capacity()`. **L10** `dwSize` is computed
before the process snapshot opens, so no early return can leak the handle.
**L11** the old-downloads property test folds in the `owner_process_running` axis
it claimed to cover, and asserts its own case count.

**A2 — docs corrected so the `%WINDIR%` contradiction cannot return.** Module
spec 02's pack table says InfoOnly and drops the two `%WINDIR%` locations that
were never shipped; `RULES-FORMAT.md` distinguishes a rule that *proposes an
action* on a protected path (a test failure) from an informational one
(permitted, and structurally unable to act); the pack's `reason` names the space
at stake and points at both Windows tools that will reclaim it.

**Decisions.** The justification gate verifies what a module *recorded* — every
shipped module records one, but making that part of the `ToolModule` contract is
a phase-3 change. The duplicate re-hash is full, never sampled: sampling would
re-admit the crafted collision the scan exists to reject. The rule re-check does
not re-test path globs (that is gate 2's question, asked properly) nor the
cleaner's `MIN_QUIET_HOURS` (a module scan policy, not a rule criterion, and
subsumed by every shipped rule's `min_age_hours`). The pack `reason` points at
the size the UI already shows rather than interpolating a byte count, because
rule reasons are static data shipped verbatim and placeholders would be a
rule-pack format change. All recorded in `docs/DECISIONS-NEEDED.md`.

**Deferred.** Nothing new. The phase-3 deferrals above still stand.

---

## 2026-08-06 — Phase 3: disk map, startup manager, real dashboard

**Treemap** (`docs/modules/04-treemap.md`). `scan` walks a drive once and rolls
every directory's size up bottom-up, keeping directory rows only — a drive with
two million files has perhaps fifty thousand folders, so the model fits in
memory and in SQLite. `children` then serves one folder at a time: it re-reads
that single directory so names are current and takes subfolder totals from the
aggregate. Spec 04 offers streaming events *or* `get_children` and says to pick
one; this is `get_children`, with no streaming path, because two ways to build
one picture is two pictures that can disagree.

Four refusals to round off: a junction is drawn hollow and never counted, so its
target's bytes appear once where they live; a folder that cannot be read is
hatched and marked, never drawn as empty; sizes are logical and the level says
how many files below it are compressed or sparse; and a cached picture states
when it was taken. Canvas renderer, hand-rolled squarify in TypeScript with
geometry tests under `node --test`. Cross-links ship: Open in Explorer, Scan
with Cleaner, Find duplicates here with the folder pre-filled.

**Startup manager** (`docs/modules/05-startup-manager.md`). Registry
`Run`/`RunOnce` in both hives plus `WOW6432Node`, both Startup folders, and
logon-triggered scheduled tasks through COM `ITaskService`. Disabling mirrors
Windows: a `Run` value is never touched and a 12-byte `StartupApproved` record
is written whose first byte's **low bit** is the disabled bit (the low bit, not
the whole byte — `06`/`07` is a real variant, and comparing bytes would silently
re-enable something the user turned off); a Startup-folder shortcut is moved
into a module-managed folder through `core::quarantine`'s move primitive and
recorded in the ledger; a task gets `Enabled = false` and the wrapper has no
delete method at all. Impact tiers come from the program's size and
`rules/startup/startup-known.json`, and every place they appear says "estimated".

**Architect addition A — the justification is in the contract.** `Recommendation`
carries a `Justification`, and `validate()` refuses a mutating action whose
justification is `Informational`. Gate 1 runs that validation on every unit, so
a hand-forged struct is *failed* rather than quietly acted on, and gate 4
dispatches on the enum instead of sniffing a finding's `meta`. The old test that
pinned the gap as a deliberate decision is replaced by one proving it is closed
at both the constructor and the executor.

**Architect addition B — the honest failures are shown.**
`QuarantineStats.unreadableRows` and `PurgeReport.affectedBatches` have been
produced since phase 1 and displayed nowhere. The drawer states both, factually.

**Dashboard.** `core::history` writes one line per run to the `scans` table —
in the schema since phase 0, never written to until now. A module that has never
run says so rather than showing a zero.

**Performance — measured, not asserted.** The first run of the 100 000-file
synthetic tree took **8 678 ms** against a 3 000 ms budget. Profiling said the
cost was not the traversal: `jwalk::DirEntry::metadata()` calls
`fs::symlink_metadata`, which opens the file on Windows, once per entry.

```
jwalk enumeration alone                      64 ms
jwalk + fs::symlink_metadata per entry     3924 ms
read_dir + DirEntry::metadata()             143 ms
```

`core::scanner` is now a sorted depth-first `read_dir` walk and `jwalk` is
dropped. First paint: **868 ms** (walk 855, cache 10, first level 3), cancel
lands in **41 ms**, zoom is **0 ms**, and the 400-rectangle layout is 0.247 ms.
Run it with `cargo test --release --test treemap_perf -- --ignored --nocapture`.

**Decisions.** "Parallel walk" was measured and rejected: parallelism was worth
80 ms, the syscall 3 800 ms. That fast path has one caveat, found by a test that
started failing intermittently — NTFS updates the copy of a timestamp held in a
parent directory's index lazily, so the listing can report a modification time
*older* than the truth, and the cleaner decides what is old enough to remove
from exactly that number. `WalkOptions::accurate_timestamps` therefore defaults
to on and only the treemap opts out, so the cleaner and the duplicate finder
behave exactly as before. The cleaner cross-link carries `?from=` and says
plainly that the cleaner scans known junk locations rather than a folder you
pick, because a pre-filled path it would not honour is worse than no path. All
in `docs/DECISIONS-NEEDED.md`.

**Deferred.** Disk-type (SSD vs spinning) is not an input to the impact
estimate — the spec lists it, and the tier is labelled an estimate everywhere,
so adding it is a refinement rather than a correction. Startup-folder shortcuts
are identified by their `.lnk` path rather than the program inside them, so the
signature badge on a folder entry reports on the shortcut; resolving it needs
`IShellLink`, which is a fourth COM surface and was not worth adding to close
this phase. Long scans still run on the async runtime rather than
`spawn_blocking`, matching the shape phases 1 and 2 shipped.

## 2026-08-06 — phase 3 review remediation (H11, M16-M21, L12-L16)

**The block, and the structural answer.** `modules::startup::stash_link` moved a
Startup-folder `.lnk` and *then* wrote the row that remembered where it came
from. With a ledger write that failed — a full disk, a locked database,
`SQLITE_BUSY`, the `startup_disabled_link_origin` unique index firing — the
shortcut left the Startup folder, nothing recorded where it went, and it vanished
from the startup manager entirely. Never deleted; merely invisible, which through
the only interface the user has is the same thing (H11).

The local fix would have been to swap two statements. The finding was that the
order was a *comment* — stated in `core::quarantine`'s header, obeyed by
`Quarantine::take`, and re-derived backwards by the next module that called into
the same file. So the order is a type now:

- `core/ledger.rs` — `ledgered_move(conn, what, step, perform)`. `LedgerStep`
  has two variants and the variant fixes when the row is written relative to the
  move: `Claim` commits the row in its own transaction *before* anything moves
  and retracts it if the move then fails; `Retire` drops the row only once the
  file is somewhere the user can see without our help. Writing a row after moving
  a file into custody — the one order that is never correct — is not expressible.
- `perform` receives a `Recorded`, and `ledger::move_user_file` will not move
  anything without one. `quarantine::move_user_file` is `pub(in crate::core)`,
  so a module has no other door. All four write surfaces (`Quarantine::take`,
  `Quarantine::restore_item`, `stash_link`, `restore_link`) are callers.
- `ledger::reconcile` closes the crash window a `Retire` leaves open. Only a
  clean `NotFound` counts as evidence that a row is stale; "we could not look" is
  not, because acting on that would be the orphan arriving by the other door.

**Ledger rows are surfaced, not dropped.** `disabled_links` returns
`DisabledLinks { links, unreadable_rows, reconciled_rows }` and the scan states
the count, the way `QuarantineStats.unreadableRows` already did.
`find_disabled_link` now *errors* on a row it cannot parse instead of returning
`None` — `None` means "not held", which sent `read_entry` on to the folder and let
a re-disable collide with the unique index. That was the second route to the same
orphan.

**M16 — the fast path was not only about clocks.** With
`accurate_timestamps: true` the walk paid for `symlink_metadata` and then read
`size_bytes` from the stale listing anyway. Both figures now come from the same
`Metadata`, and `core::scanner`'s header no longer claims sizes are unaffected —
it says sizes lag on the fast path and that the treemap accepts that.

**M17 — the treemap's two path arguments are tied together.** `treemap_children`
took `root` and `path` from the WebView with no containment check: `path` under
`C:\Windows\System32\drivers\etc` with an unrelated `root` enumerated it and the
breadcrumb walked to `C:\`. Both commands run `root` through `brain::gate_path`
and `path` through `path_compare::is_inside`.

**M18 — catalog-signed is not unsigned.** `verify()` asked only
`WTD_CHOICE_FILE`, so `notepad.exe` — like almost everything Windows ships —
came back `TRUST_E_NOSIGNATURE` and was reported "unsigned, unknown publisher".
A file with no embedded signature is now hashed with
`CryptCATAdminCalcHashFromFileHandle2`, looked up with
`CryptCATAdminEnumCatalogFromHash` and verified with `WTD_CHOICE_CATALOG`; the
publisher comes from the `.cat`. Both questions answered "no" is what `Unsigned`
now means, and a catalog database that cannot be consulted is `Unknown`.

**M19 — the signature check leaves the access time alone.** It opens the binary
itself and hands the handle to `WINTRUST_FILE_INFO.hFile`. The measurement found
more than the review did: `SetFileTime`'s `0xFFFFFFFF` opt-out needs
`FILE_WRITE_ATTRIBUTES` on the handle, so a read-only open has it rejected and
the clock moves anyway. Every stage — open, read, `WTD_CHOICE_FILE`, catalog
hash, `WTD_CHOICE_CATALOG` — was measured; all five hold the clock now.

**M20** — the folded tail row is `sized: tail.iter().all(|row| row.sized)`.
**M21** — the treemap's arena is keyed on `(parent slot, one normalised name)`
instead of a whole canonical path string, and resolves through
`Path::strip_prefix` instead of `same_path`/`is_inside`. A 3 000-deep tree went
from 75.3 s around a 5.34 s walk (14×) to 5.7 s around 4.7 s (1.2×).

**L12** an entry whose metadata call failed is recorded in `WalkSummary::unreadable`
rather than dropped. **L13** `approval_payload`'s doc now matches the code (seven
bits of byte 0, not bytes 1-3). **L14** `read_entry` computes `os_owned` for
Startup-folder entries the way `describe` does. **L15** `Recommendation`
deserialises through `#[serde(try_from)]`, so `validate()` runs at every
construction site. **L16** `cancelling_a_big_walk_stops_it_promptly` is no longer
`#[ignore]` — it builds a cheap 12 000-file fixture of its own — and
`tests/treemap_perf.rs` gained a deep-tree case for the axis M21 lived on.

**Deferred.** A "move in flight" column on both ledger tables would close the
`Retire` crash window without a filesystem read; it needs a migration and is
recorded in `docs/DECISIONS-NEEDED.md`. The M16 lag could not be reproduced on
this machine's volume — the test asserts the invariant that makes it impossible
and prints both sources so a run says which regime it was in.

---

## 2026-08-06 — every command was dead at runtime; the suite never looked

**The bug.** `lib.rs` registered `Arc<Database>` (phase 3 introduced the `Arc` so
`StartupControlAdapter` could hold a clone) while `commands.rs` still asked for
`State<'_, Database>`. Tauri resolves managed state by `TypeId`, so those are two
unrelated keys: every one of the nineteen database-taking commands failed on its
first invocation with `state not managed for field `db``. The app was completely
non-functional and `cargo check`, `cargo clippy` and 486 unit tests all passed —
the mismatch does not exist until runtime.

**Fixed.** `commands.rs` now spells the parameter `State<'_, Arc<Database>>` in
all nineteen places; the bodies were untouched because `Deref` carries `db.with`
and `&db` through. The audit that followed found two more of the same mistake in
`lib.rs`, both outside the IPC path and both silent: `clear_persisted_timer` and
the tray's `Cancel timer` asked `try_state::<Database>()` and got `None`, so a
fired timer stayed in the settings table to re-arm on the next start and the tray
menu item refused outright. All four out-of-band call sites now go through one
`managed_database()` accessor, so there is a single spelling to get right and it
sits next to the `app.manage` call that decides it. Full audit: seven managed
types, seven consumers, all matching.

**The structural half.** `tests/command_wiring.rs` builds a real Tauri app on
`tauri::test::mock_builder()` and invokes **every** registered command through
`get_ipc_response`. It asserts two things per command: no response says
`state not managed` (the bug), and none says `invalid args` (which would mean the
probe's arguments stopped at deserialization and never reached the state lookup,
i.e. the probe proves nothing). Against the broken code it fails 19 of 26 with
the user's verbatim error — and the `invalid args` guard fires too, on the one
probe whose payload depends on another command having worked.

Coverage is enforced rather than maintained: `run()` and the test share
`manage_state()` and `invoke_handler()`, so the test cannot register a different
app than production; and two source-parsing tests hold three lists in step — the
`#[tauri::command]` functions, the `generate_handler!` list, and the probe table.
A new command that is never registered fails one; a registered command with no
probe fails the other.

**Decisions taken.**

- `State<'_, Arc<Database>>` rather than unwrapping the `Arc`: the startup
  adapter genuinely needs a second handle, and it matches the `Arc<ShutdownTimer>`
  parameter already in the file.
- `tauri = { features = ["test"] }` under `[dev-dependencies]`. Resolver 2 keeps
  the feature out of `cargo build --release`.
- `build.rs` now embeds a ComCtl32 v6 manifest into **test** binaries
  (`cargo::rustc-link-arg-tests`). Tauri's default features pull in `muda`, which
  imports `SetWindowSubclass` and friends — exported only by ComCtl32 v6, which
  Windows hands over only to a process whose manifest asks for it. `tauri-build`
  manifests the app binary; nothing manifested a test binary, so any test target
  linking Tauri's runtime died at *load* time with `STATUS_ENTRYPOINT_NOT_FOUND`
  before a single test function ran.

**Verified against the running app**, not just the log's startup lines: the
shipping binary (`tauri build --no-bundle`, so the WebView loads
`tauri.localhost` rather than `devUrl`) was launched with `--dry-run` and driven
through its own `__TAURI_INTERNALS__.invoke` over WebView2 CDP. Twelve commands
returned real payloads, and the app's own log recorded `timer armed`,
`timer cancelled`, `startup scan finished entries=44` and a full cleaner
`scan finished` — all from commands that take the database.

**Deferred.** The probe table's arguments are chosen to be refused early and
cheaply; the test proves dispatch and state resolution, not command behaviour.
Behaviour stays the job of the unit tests.

---

## 2026-08-07 — App icon, and the folder finally matches the name

**Built.** No code change — assets and housekeeping.

- The final logo (folded pocket knife with a Windows-flag inlay, flat blue
  circle) landed as `assets/logo.png`, with the GitHub/social variant kept
  beside it as `assets/logo-gh.png`. `tauri icon assets/logo.png` regenerated
  every icon in `src-tauri/icons/` (`icon.ico`, `icon.icns`, `icon.png`, the
  `Square*`/`StoreLogo` set, 32/64/128 px); the iOS/Android outputs it also
  emits were deleted — this project has no mobile target. `static/favicon.png`
  is the 128 px render of the same logo. `tauri.conf.json`'s `icon` list was
  already correct and is untouched.
- The working directory was renamed `T:\Claude\Cleanify` → `T:\Claude\WinKnife`
  (done outside a session; the session lock had blocked it earlier). Verified
  after the move: the repo itself contains no absolute-path references to the
  old location, the git remote already points at `armadavalor/WinKnife`, and
  the only remaining `cleanify` mentions in the tree are the historical rename
  entries in this changelog, which stay.

**Deferred.** `npm run tauri build` was not run for the icon swap; the embedded
`.ico` will be picked up by the next bundle build.

---

## 2026-08-07 — The mount effect that never stopped mounting

**Built.** A UI-only fix for a loop that made three screens look permanently
broken. Nothing under `src-tauri/` was touched.

**The failure.** Every page mounted its data with `$effect(() => { void
store.load(); })`, and every load-like store method opened with `if (this.status
=== "loading") return;`. That guard is a *tracked read* performed synchronously
inside the effect, so the effect subscribed to `status` — and the very next line
wrote `status`, which re-ran the effect, which called `load()` again. A
self-feeding loop that ran for as long as the page was mounted.

Visible symptoms: Settings sat on "Loading settings…" forever with the settings
already fetched; Disk map sat on "Looking for drives…" with two drives and a
selection already in the store; the Dashboard's "Could be reclaimed" tile showed
an em dash because `status === "ready"` was essentially never observable; and
the whole thing hammered the Tauri IPC channel in the background for the life of
the window.

**The fix, two layers.**

- *Stores* — the re-entry guard is now a plain (non-`$state`) in-flight promise
  field, so it cannot be observed by the effect graph. `settings.load`,
  `dashboard.load` and `treemap.loadDrives` wrap an unchanged private
  `#doLoad`/`#doLoadDrives`; `modules.load` and `startup.load` additionally keep
  a plain `#loaded` flag to preserve their "already fetched, don't refetch"
  semantics, cleared when a fetch fails. Every `status` transition, error path
  and public signature is unchanged. `quarantine.reload`, `scan.scan`,
  `duplicates.scan` and `shutdown.refresh` were audited and left alone — they
  never had a state-reading re-entry guard. `treemap.scan`/`treemap.open` guard
  on `scanning`/`zooming` (also `$state`) but are reachable only from click and
  key handlers, never from an effect, so they were left as they are.
- *Call sites* — run-once mounting is no longer reactive at all. `+layout`,
  dashboard, settings, cleaner, duplicates, startup, treemap and shutdown-timer
  now mount their data in `onMount`. The three effects that exist to install a
  backend listener with a teardown stay effects, but their setup call is wrapped
  in `untrack(...)` so nothing read on the way in can re-trigger them. The
  duplicates `?root=` effect, ConfirmDialog's focus effect and treemap's
  ResizeObserver/repaint effects are intentionally reactive and untouched.

**Verified.** `npm run check` (176 files, 0 errors, 0 warnings), `npm run build`
and `npm run test:ui` (12/12) all pass. The live-app probe — Settings and Disk
map reaching a settled state, the Dashboard tile showing a figure, and IPC
traffic going quiet after mount — is the reviewer's, not the implementer's.

**Review + probe (architect).** Adversarial review of the diff found no
CRITICAL/HIGH issues. Live probe against the running dev app confirms the fix:
Settings renders the pack list, Disk map settles into the drive picker, the
Dashboard tile shows a real figure and stays there. One MEDIUM, pre-existing
and deferred: the Startup page's Refresh button is not disabled while a toggle
is in flight, so `reload()` can run concurrently with the `reload()` a
`disable()`/`enable()` issues internally — two enumerations race and the last
response to land wins. Fix belongs with a small sequencing pass over
`startup.reload()`, not in this diff.

## 2026-09-13 — Startup race, cleaner history and release preparation

- Fixed overlapping startup enumeration/mutation requests and added a deferred-response regression harness to test:ui.
- Cleaner now shows the saved previous scan estimate without restoring stale actionable findings; returning to idle reloads the summary.
- Production IPC fallback and narrow-window overflow were not reproduced; no speculative workaround. See POLISH-QA.md for coverage and remaining cases.
- Updated English/Turkish README, added release checklist, portable ZIP/NSIS/checksum packaging and a draft-release workflow. Optional LLM work, demo GIF, external uploads/publication and fresh-VM acceptance remain pending.
- Type check, UI tests and Clippy pass. Rust: 481 unit tests passed without opt-outs; 5 link safety tests require Windows symlink privileges. Remaining suites completed with the explicit local link-test opt-out; those five are not counted as verified.

- Local release artifacts built: NSIS installer and portable ZIP containing exe + 11 rules resources + license/readmes, with SHA-256 file. No GitHub publication.
- Graphify snapshot created: 107 code/config files + 26 documents, 2181 nodes / 6374 edges / 79 communities. Health caveat: 276 unresolved AST endpoints plus duplicate relationships; source verification remains mandatory. No new paid API, watcher or Git hook. See DEVELOPMENT-TOOLS.md.

## 2026-10-03 / 2026-10-11 — Phase 4 release audit fixes

Before the first release, a read-only audit checked the hard rules: raw
deletes, network, read-only scans, protected paths and reasons. It found two
violations and three items that needed review. Startup ratings, the content
security policy and log retention landed as separate commits (`93fc13b`,
`a2057ec`, `8bccbae`). The recycle bin was escalated and the owner chose the
narrow exception on 2026-10-11 (`97d464f`). The LLM layer moves to v0.2.0,
so v0.1.0 ships with no network client. Decisions are in DECISIONS-NEEDED,
"Phase 4 release audit — 2026-10-03".

**Built (this entry's commit): quarantine cross-volume moves no longer
remove the source by path after verification** (security review C, C2, D).
- Every source file is pinned with a read-and-delete handle that lets others
  only read; every folder is opened as itself, never through a link, and
  held without delete sharing. A top folder swapped for a junction mid-move
  is refused, and everything must resolve inside the top folder's real path.
- The copy reads through the pins and is checked byte for byte, including
  Zone.Identifier, the named-stream list and the attribute mask (read-only,
  hidden, system, archive, not-indexed). Any other alternate data stream is
  refused before anything is created.
- Exactly the pinned files are deleted through their handles; streams are
  re-listed after the delete is marked and the delete is taken back if they
  changed. Folders go deepest first and only when empty, so anything that
  arrived during the move stays and is logged.
- Once any source file is gone the move reports success and keeps its
  ledger row. If nothing was deleted, the destination is cleaned only by
  file identity and empty-folder removal, never `remove_dir_all`.
- `rename_no_replace` takes `\?\` paths, so moves past MAX_PATH work.
- The handle code lives in `core/platform/pinned.rs`. Attribute setters stay
  in `quarantine.rs` so the `no_other_module_removes_user_data` guard holds.
- 11 new tests (zone identifier, other streams refused, late stream keeps
  its file, attributes, junction swap, containment, long path, read-only
  cleanup, stream listing, final path).

**Also.** CI's stable Rust moved to 1.99, whose clippy flags `chunks_exact(2)`;
`registry.rs` and `pinned.rs` decode UTF-16 with `as_chunks` (`3a1e007`).

**Review.** Opus adversarial review of the first patch: SHIP-WITH-FIXES
(stale probe tests, named streams lost, junction swap of the top folder,
attributes, long paths). All fixed. Second Opus review: SHIP. An NTFS probe
confirmed the delete take-back and that a marked file cannot be reopened.
The four LOW items it left are now closed:
- A take-back is confirmed through the same handle (`DeletePending`). An
  unconfirmed one counts as a delete, so the verified copy stays and the
  path is logged. Before the copy is undone, the source pin is closed and the
  source is checked to still be there. A test where the take-back is skipped
  used to lose both the source and the copy.
- EFS-encrypted files and folders are refused across drives: the copy would
  be written in the clear. The encryption loss itself predates this work. A
  same-volume rename still works.
- Files with more than one name (hard links) are refused with that reason.
  Before, a tree holding two names of one file was moved as two separate
  copies.
- The guard test also rejects delete-on-close (by name and by value),
  `NtSetInformationFile`, `DeleteFileA`, `RemoveDirectoryA`,
  `SHFileOperation`, `IFileOperation` and `MOVEFILE_REPLACE_EXISTING`. Its
  exemption is now the exact path `src/core/quarantine.rs`, and
  `open_to_delete` and `open_dir_to_delete` are `pub(in crate::core)` and
  usable only from quarantine.
- 5 new tests:
  - an unconfirmed take-back keeps the copy;
  - a real take-back during `take` leaves the source, quarantine and ledger
    clean;
  - the pending flag primitive;
  - EFS refused but renamed;
  - hard links refused.

  The EFS test needs an EFS-capable volume. On first run it creates the
  user's EFS certificate, and it runs `cipher` only inside its own temp
  directory.

**Follow-up review (2026-10-12).**
- Warnings reach the user. A cross-volume move that happened but left
  something undone now returns warnings (path and plain-English reason).
  They go through `move_path` into `TakeOutcome`, `ExecItemResult.warnings`
  and `RestoreOutcome.warnings`, and are shown in amber under the cleaner,
  duplicates and quarantine-drawer results and in the startup note. The
  fields are optional and omitted when empty; there is no ledger change.
  `take()` keeps its signature, and `take_reporting()` returns the
  warnings.
- After a refused delete, only "not found" or another file at the name
  means the source went (`try_file_identity`). An unreadable identity
  keeps both (round 2 turned its failure into a warning).
- A late stream whose take-back fails is no longer counted as moved.
  The copy stays and a warning says the stream was lost.
- A restore refused because the item is EFS-encrypted says it stays held
  and to decrypt it where it is held (purge policy: see round 2, item 8).
- Guards:
  - `MoveFileEx` is a guard-test needle.
  - `OpenOptionsExt::custom_flags` is disallowed in `clippy.toml`, and
    its five uses in `pinned.rs` each carry a visible `allow` with a
    comment.
  - The `open_to_delete` and `open_dir_to_delete` docs now say the
    compiler limits them to `crate::core` and the guard test limits them
    to `quarantine.rs`.
- Tests: 9 Rust (partial take and partial restore warnings, unreadable
  identity, each revoke fix on its own, late stream lost, EFS restore
  refused and held, wire format, `NotFound` identity) and 5 UI
  (`execNotes`). Each revoke test was checked to fail with only its own
  fix reverted.

**Follow-up review, round 2 (2026-10-12).**
1. When the source's identity cannot be read after a refused delete, both
   copies are kept and the move now succeeds with a "both kept" warning,
   so the ledger row of the moved copy survives (take and startup stash).
2. Startup move warnings travel through disable (`StartupControl::disable`)
   and re-enable (`startup_enable`) into the startup page's warnings.
3. `MoveWarning.path` is a lossy `String`.
4. `MoveFailure::Encrypted` is a typed variant; nothing reads the message.
5. A take-back that cannot be confirmed has its own reason, separate from
   a lost stream.
6. `reclaimed_bytes` leaves out bytes left at the old place.
7. Warning lists in the drawer, cleaner and duplicates pages are keyed by
   index, so two equal warnings no longer collide.
8. User decision b: an item whose encrypted restore was refused is never
   removed by the automatic purge. Migration 004 adds
   `restore_refused_at`; the restore sets it; `purge_expired` skips it;
   `enforce_size_cap` counts it but never evicts it and logs when it cannot
   get under the cap. The drawer labels it "restore refused: encrypted;
   kept until you remove it" and offers Remove behind a permanent-delete
   confirmation (`quarantine_purge_item`, which refuses unmarked items).
   Details in DECISIONS-NEEDED.
- Tests: Rust for each item (both-kept take and startup stash, startup
  warnings end to end, typed encrypted failure, unconfirmed take-back,
  reclaimed bytes, retention purge keeps refused, size cap skips refused,
  size cap removes nothing when only refused items are over, user removal,
  removal refused for unmarked items, version-3 database migrates), the
  command-wiring probe for `quarantine_purge_item`, and UI (`execNotes`:
  `warningsNote`, `isRestoreRefused`; startup regression: re-enable
  warnings).

**Follow-up review, round 3 (2026-10-12).**
1. If marked (refused-restore) items alone are at or over the size cap,
   `enforce_size_cap` removes nothing for size and logs a warning.
   Retention still applies, and eviction is unchanged while they are under
   the cap. `QuarantineStats` gains `refusedBytes` and `capBytes`; the
   drawer says in plain English why nothing is removed for size.
2. Fixed the runs of spaces in `STREAM_UNCONFIRMED`.
3. A failed Remove in the drawer gets its own message
   (`quarantine.removeFailed`), not the purge's "retried on the next
   purge".
4. The quarantine path of a refused item is shown as selectable text, both
   in its drawer row ("Held at") and in the Remove confirmation
   (`ConfirmDialog` gains an optional `detail`).
5. `mark_restore_refused` reports whether it marked the row. If it did
   not, the refusal message drops "kept until you remove it" and says the
   purge may still remove the item, and when.
6. The startup stash "both kept" arm, which leaves the original shortcut
   running at logon, is recorded in DECISIONS-NEEDED. Code unchanged.
7. Tested that `purge_item` on a restored item returns
   `UnknownQuarantineItem`.
8. `run_migrations` refuses a database from a newer build
   (`CoreError::SchemaTooNew`, `schema_too_new`). Setup shows a native
   error box (`core::platform::show_fatal_error`, gated on armed OS
   effects) instead of exiting silently. The windows crate feature
   `Win32_UI_WindowsAndMessaging` was added.
- Tests (Rust): size cap removes nothing while refused items fill it, size
  cap still evicts when they are under it, removing a restored item is
  refused, the refusal promises to keep only when marked, a failed mark
  (SQLite trigger) is reported, no warning reason has runs of spaces,
  wire format with the new stats fields, a newer database is refused, and
  a test build never shows a box. UI (`execNotes`): `sizeCapBlocked` and
  `removalFailure`.

**Follow-up review, round 4 (2026-10-12).**
1. The size cap applies to unmarked bytes only (`over_by = unmarked - cap`).
   Marked items are never counted and never evicted. This replaces round
   3's "remove nothing once they fill the cap", which left a cliff just
   below the cap. It narrows decision b as written, so it is flagged for
   architect review in DECISIONS-NEEDED. `QuarantineStats` gains
   `refusedItems`. The drawer explains whenever any marked items are held
   (`quarantine.refusedHeld` replaces `capBlocked`; `refusedHeld`
   replaces `sizeCapBlocked`).
2. `restore_item`'s "kept" promise is tested where it is wired. The EFS
   harness runs with the `refuse_marker` trigger, and the error must say
   the purge "may still remove it". Hard-coding `true` at the call makes
   it fail.
3. Fixed the runs of spaces in the size-cap and stream-unconfirmed log
   messages (backslash continuations). A new test scans `quarantine.rs`
   for a run of spaces inside a line.
4. A failed Remove is reported by kind (`RemovalFailure`):
   - `stuck`: the file could not be deleted. Only this case says "still
     held, never auto-purged".
   - `notEligible`: `unknown_quarantine_item`, or `quarantine` for an
     unmarked item. The drawer says so and asks for a refresh
     (`quarantine.removeRefused`).
   - `error`: anything else, shown as it came (`quarantine.removeError`).
   `quarantine.removeFileStuck` was removed.
5. `manage_state` logs the `db::open` error while the log guard is still
   alive.
- Tests:
  - Rust:
    - Marked items just under the cap do not evict what fits.
    - Marked items are not counted against the cap (it replaces "removes
      nothing while refused items alone fill it").
    - The skip and under-the-cap tests now expect unmarked-only eviction.
    - The wiring test, the source runs-of-spaces test, and `refusedItems`
      in the wire format.
  - UI: `refusedHeld`, and `removalFailure` by kind.
  - Not covered: no test covers the `db::open` log line.

**Verified (Rust 1.99, 2026-10-12, round 2).** `cargo fmt --check` clean;
`cargo clippy --all-targets -- -D warnings` clean; `cargo test`: `540
passed; 5 failed` in the library, and the 5 are the symlink tests that need Developer
Mode or elevation (they run on CI); integration suites 8 + 3 + 3 + 1 passed,
3 ignored. UI: `svelte-check` 0 errors, 0 warnings; `test:ui` 22 passed plus
the startup regression; `npm run build` done.

**Verified (Rust 1.99, 2026-10-12, round 3).** `cargo fmt --check` clean;
`cargo clippy --all-targets -- -D warnings` clean; `cargo test`: `548
passed; 5 failed` in the library, and the 5 are the same symlink tests that need
Developer Mode or elevation; integration suites 8 + 3 + 3 + 1 passed, 3
ignored. UI: `svelte-check` 178 files, 0 errors, 0 warnings; `test:ui` 24
passed plus the startup regression; `npm run build` done.

**Verified (Rust 1.99, 2026-10-12, round 4).** `cargo fmt --check` clean;
`cargo clippy --all-targets -- -D warnings` clean; `cargo test`: `551
passed; 5 failed` in the library, and the 5 are the same symlink tests that
need Developer Mode or elevation; integration suites 8 + 3 + 3 + 1 passed, 3
ignored. UI: `svelte-check` 178 files, 0 errors, 0 warnings; `test:ui` 25
passed plus the startup regression; `npm run build` done. The drawer's new
texts were not checked in a running app.

**Final review (round 4): SHIP.** The one LOW finding, that the Settings label
for the size limit no longer told the whole truth, is fixed with a hint under
the field: encrypted items kept after a refused restore are not counted against
it. Two notes are left open. The "not eligible" remove message keys off the
catch-all `quarantine` error code; that is accurate today, but a dedicated code
would make it robust. Setup errors after `db::open` still miss the log file.

**Deferred.** The startup page still shows the known-app label rather than
the rating (DECISIONS-NEEDED, note 3). Clean-VM acceptance before the v0.1.0
tag.

---

## 2026-10-12 — Phase 5 task 0: issue #21 follow-ups

**Built.**
- **Startup "both kept" arm, option (b).** A held shortcut whose original name
  is occupied again is listed once (no duplicate id), as off, with a new
  `leftoverShortcut` field; the page labels it "Off — but its shortcut is still
  in the Startup folder" with a reason naming the path (EN + TR). Re-enable
  compares the two files with `content_hash::full_hash`: byte-equal → the held
  copy goes to the quarantine and the row is forgotten, so the entry reads as
  enabled; different or unreadable → the Occupied refusal, now saying which.
  `StartupService` takes the `Quarantine` to do this. Tests for both arms and
  for the single, flagged listing.
- **`quarantine_item_not_marked`.** `purge_item` on an unmarked item has its own
  error code; the drawer maps it to its own message.
- **Log guard.** `manage_state` puts the appender guard into managed state
  before `db::open`, so setup errors after it reach the log file
  (`tests/log_guard_survives_setup_error.rs`).
- **Turkish.** A partial `tr` dictionary for the phase 5 strings (fallback to
  English, no switcher yet).

**Decisions.** EFS cap (round 4) closed with no code change; downgrade
mitigation closed as won't-fix. Recorded in `DECISIONS-NEEDED.md`, plus a
pre-existing camelCase/snake_case mismatch in the `StartupEntry` TS mirror,
found in passing and not fixed.

## 2026-10-12 — Phase 5 task 1: single instance and `--module`

**Built.**
- **`tauri-plugin-single-instance`**, registered first. A second launch
  forwards its argv to the running app and exits before any `setup`, so it never
  opens the database or re-arms the persisted timer. The running app's
  `handle_second_instance` shows, un-minimises and focuses `main` (also out of
  the tray), then opens the requested module.
- **`--module <id>` / `--module=<id>`.** `parse_launch_args` (pure, next to
  `is_dry_run`) and `launch_route` (registry-validated; unknown or empty id ->
  warn log + `/dashboard`, no dialog). The route is parked in
  `PendingLaunchRoute` and pulled by the shell with `take_launch_route` on mount
  and on each `launch:navigate`, so the first launch's route is not lost before
  the WebView listens, and a route opens once.
- **Issue #3 regression** (`tests/single_instance.rs`): a timer persisted
  before start is re-armed once by `manage_state`; repeated forwarded launches
  leave the armed timer and the stored row untouched.

**Not verifiable here.** The plugin's cross-process forwarding (a real second
`winknife.exe`) needs a manual run; the tests drive `handle_second_instance`
directly on a mock app.

## 2026-10-12 — Phase 5 task 5: restore point module

**Built.**
- **Elevated helper** (`core/platform/restore_point.rs`). The app re-launches
  its own exe through `ShellExecuteExW` (`runas`) with
  `--create-restore-point "<desc>"` and waits up to 5 minutes for the exit
  code. The helper is recognised at the top of `run()` (`launch_mode`), before
  the database, logging-to-DB, the Tauri builder and the single-instance
  plugin, so it never forwards to or blocks on the running app. It calls
  `SRSetRestorePointW` (`BEGIN_SYSTEM_CHANGE` / `MODIFY_SETTINGS`, then
  `END_SYSTEM_CHANGE`) and compares the newest WMI `SystemRestore` sequence
  before and after to tell created / skipped (24h) / may-have-skipped. Exit
  codes are shared constants. No PowerShell; System Protection and
  `SystemRestorePointCreationFrequency` are never touched.
- **Module** (`modules/restore_point.rs`, `has_scan: false`). Read-only
  protection status (on / off / unknown, with a reason), description sanitising
  (printable only, 64 chars, default "WinKnife — before changes"), exit-code to
  outcome mapping, `RestorePointPlatform` trait with a fake; test processes can
  never reach the real helper. Commands `restore_point_status`,
  `restore_point_create`, `restore_point_open_settings`, all covered by
  `tests/command_wiring.rs`.
- **UI.** `/restore-point` page (status, description, UAC notice before the
  button, one sentence per result, System Protection button only when it is
  off), sidebar icon, EN + TR strings, and a plain "Create a restore point
  first" link on the startup page.

**Not verifiable here (manual, admin VM or local).** Create succeeds; a second
create within 24h reports skipped; answering "No" to UAC reports declined. None
of these has been run: the UAC prompt and `SRSetRestorePointW` cannot be
exercised from the test suite.
## 2026-10-12 — Phase 5 task 2: keep-awake

**Built.** Module `keep-awake`: hold the PC awake for 1 to 1440 minutes,
optionally with the screen on.

- `core/platform/execution_state.rs`: `ExecutionState` trait,
  `RecordingExecutionState` fake and the crate-private `SystemExecutionState`
  (`SetThreadExecutionState`, refused unless `real_os_effects_armed()`).
- `modules/keep_awake.rs`: one dedicated worker thread per session (the OS flag
  is per thread), wall-clock deadline re-checked every 30 s so a sleep/wake
  cannot stretch the session, stop joins the thread, replacing a session starts
  a fresh thread. Max 24 h, no forever mode, nothing persisted. Commands
  `keep_awake_start/stop/status`, event `keep-awake:changed` (no payload).
- UI `src/routes/keep-awake/`: presets 30m/1h/2h/4h plus custom, screen toggle,
  always-visible Stop, countdown from the deadline, "nothing persisted" line,
  hint when a shutdown timer is armed. Sidebar icon `coffee`. EN keys and the
  first `tr` dictionary (keep-awake keys only so far; the rest falls back to EN).
- Wiring test probes cover the three commands (dry run: recorder, no OS call).

**Decisions.** Validation errors reuse `CoreError::InvalidTimer`.

**Deferred / unverified.** The manual `powercfg /requests` check on a real
Windows session was not run. 5 pre-existing tests fail in this environment
because creating symlinks needs Developer Mode or elevation (executor,
quarantine x2, scanner, cleaner); none involve this module.

## Phase 5 - empty-folders module (spec 08)

**Built.**
- `src-tauri/src/modules/empty_folders.rs`, rule pack `rules/empty-folders.json`.
  A folder is empty only when it holds no file at any depth (even `desktop.ini`
  counts). Only the topmost empty folder of each subtree is reported, with the
  nested count in `meta`. Reparse points, cloud placeholders, unreadable folders
  and protected trees are never traversed or reported (the last two are counted).
  A drive root may be scanned but is never a finding. Cancellable, with progress.
- Analysis: hard gates first (`path_refusal`, protected-below, system folder),
  then the rule pack via the Brain. Savings are 0. Execute is `Quarantine`; the
  executor re-checks emptiness (`recheck_refusal`) and skips with a reason.
- Command `start_empty_folder_scan(root?)` (default root: user profile).
- UI `src/routes/empty-folders/` plus `stores/emptyFolders.svelte.ts`, `folder`
  icon, EN + TR keys, typed wrappers and mirrored types. Nothing starts selected.
- Brain: `FindingKind::Directory` branch; executor: lock probe skipped for
  directories. Cleaner now only loads its own pack (`CLEANER_MODULE_ID`), and
  `read_settings` filters packs by module.

**Decisions.** See DECISIONS-NEEDED.md (2026-10-12 phase 5 entries).

**Deferred / unverified.** The access-denied test returns early when the
`icacls` deny is ineffective (e.g. elevated token), so it may not exercise that
path on every machine. The same 5 environmental symlink test failures apply.
## 2026-10-12 — Phase 5 task 3: File Unlocker (`file-unlocker`)

**Built.** "Which program is holding this file open?", read-only, with one
polite action.

- **Restart Manager wrapper** (`core/platform/restart_manager.rs`). An
  `RmApi` trait (the real `SystemRmApi`, plus `FakeRmApi` for unit tests)
  under `RmSessions`, which implements the module-facing `RestartManager`
  trait. `RmEndSession` always runs through a `Session` drop guard, also
  after a failed registration. `RmGetList` retries `ERROR_MORE_DATA` at
  most 3 times. Files are registered in batches of 500.
- **Module** (`modules/file_unlocker.rs`). Files and folders expand to at
  most 5 000 files. The scanner walker does not follow links or reparse
  points, and a link given as a root is skipped with a reason. `capped`
  drives the "only the first 5 000 files were checked" note. Every row
  carries a reason and a stable `reason_code`.
- **Attribution choice.** With 50 files or fewer, the query runs one RM
  session per file, so each row lists what it holds (the first 3 plus a
  count). Above 50, it runs one session over all the files and the row
  says "holds one or more of these files". Per-file sessions for thousands
  of files would be slow, and a wrong attribution is worse than none.
- **Ask to close** (`unlocker_request_close(pid, start_time)`).
  - Offered only for window and console apps.
  - Info-only, with a reason and no button: services, Explorer,
    `RmCritical`, unknown kinds, PID 4 and WinKnife's own PID. Rust
    refuses them again server-side.
  - A start-time mismatch (PID reuse) is refused.
  - The close opens a fresh session that registers only that
    `RM_UNIQUE_PROCESS` and calls `RmShutdown` with flags = 0. It never
    uses `RmForceShutdown`, `TerminateProcess` or handle closing.
  - Afterwards the process is checked again: closed, still running,
    already exited or refused. Every request is logged with the exe path
    and the outcome.
  - The real `RmShutdown` is gated on `real_os_effects_armed()`, so no
    test can close anything.
- **UI** (`routes/file-unlocker`).
  - File and folder pickers (`tauri-plugin-dialog`, open only), window-wide
    drag-drop and a typed path.
  - Confirm dialog: "<App> will be asked to close. If it has unsaved work,
    it may ask you to save first."
  - The query runs again after a close.
  - "Open file location", and an empty state with the
    read-only/permissions hint.
  - EN keys plus a partial TR dictionary.
  - The start time travels as a decimal string, because it does not fit a
    JS number.
- **Tests.**
  - Fake-driven unit tests: classification, batching, bounded retry, the
    guard on failure, fresh-session close, PID reuse, server-side refusals.
  - `tests/unlocker_query.rs`: a PowerShell child holds a temp file with
    no sharing, and the test asserts that the real query names its PID.
  - The command-wiring test probes both commands.

**Deferred / unverified.** The manual check against Word with unsaved work
(the save prompt and the "still running" path) has not been done. There is
no language picker for TR yet (see DECISIONS-NEEDED).

---

## 2026-10-12 — v0.2.0 integration

**Built.** Merged `feat/v0.2.0-core` (#21 follow-ups, single instance +
`--module`, restore point), `feat/v0.2.0-tools` (keep-awake, empty folders) and
`feat/v0.2.0-unlocker` (file unlocker) into `feat/v0.2.0`.

- **Folder gate.** The executor now refuses a `Directory` finding from any
  module other than empty-folders: folders skip the lock probe, and only that
  module re-checks emptiness right before the move. Test:
  `a_folder_finding_from_another_module_is_refused`.
- **EFS tests are opt-in.** The three quarantine tests that run `cipher /e`
  make Windows create an EFS key and show a "back up your key" notification on
  a contributor's machine. They now run only with `WINKNIFE_EFS_TESTS=1` and
  print a visible skip line otherwise; `ci.yml` and `release.yml` set it.
- `cargo fmt` over the merged tree.

## 2026-10-12 — UI declutter

**Built.** Less visible prose, same information.

- New shared components `PageHeader`, `InfoTip`, `StatCard`, `EmptyState`.
  Every page subtitle and long helper paragraph now sits behind an (i)
  popover (hover, focus or click; Escape closes).
- Dashboard: stat cards via `StatCard`; module cards in a 1/2/3-column grid
  with the description in an InfoTip.
- `FindingList`: each reason shows as one line and expands on click, so
  every recommendation still carries a readable reason.
- Cleaner: last-scan summary and Scan button merged into one row. Keep Awake:
  idle state is a single line; the big countdown appears only while active.
- Empty "nothing found" states use `EmptyState`. All pages share one
  container width (treemap stays wider).
- Kept visible on purpose: warnings, errors, the permanent recycle-bin note,
  ConfirmDialog content, status lines and short safety hints.

## 2026-10-13 — Startup: OS-owned toggles were clickable

**Fixed.** `StartupEntry` serialises with `rename_all = "camelCase"`, but
`src/lib/types.ts` and the startup page read `os_owned`, `os_reason`,
`signature_note` and `impact_reason`. All four arrived as `undefined`, so
Windows-owned rows looked clickable, their reason never showed and the
signature/impact tooltips were empty. The backend still refused the toggle
(`executor.rs` live re-check), so nothing was ever disabled wrongly; UI only.

- TS fields renamed to camelCase; new Rust test
  `the_wire_format_is_camel_case_as_the_ui_reads_it` pins the JSON key names.
- The OS reason is hidden when it repeats the known-app note word for word.
- Live check (CDP, dev build): 45 rows, 26 OS-owned, all greyed and disabled.

**Deferred (noticed during the check).** `%windir%`-style commands (e.g.
SecurityHealth) show "unsigned / not a program on this machine", so env vars
look unexpanded before the signature check. The sidebar footer still says
"Phase 3 — disk map + startup" (`status.phase`), so it should change with the 0.2.0 bump.

## 2026-10-13 — Version 0.2.0

**Built.** Version 0.2.0 in `package.json`, `package-lock.json`,
`Cargo.toml`, `Cargo.lock` and `tauri.conf.json`. The sidebar footer showed the
stale "Phase 3 — disk map + startup"; it now shows `Version {version}` read
from `package.json` (EN + TR). `CHANGELOG.md`, both READMEs and
`docs/RELEASING.md` describe 0.2.0 with nine tools.

**Decisions.** 0.1.0 was never tagged, so 0.2.0 is the first public release and
its changelog section carries the former 0.1.0 list plus the phase 5 tools.
The startup wire-name fix is not listed under "Fixed": no released version had
the bug.

**Deferred.** Fresh-VM acceptance, VirusTotal and the demo GIF stay manual
maintainer steps (`docs/RELEASING.md`).

**Correction (same day).** `CHANGELOG.md` claimed an "English and Turkish
interface", carried over from the 0.1.0 draft. Nothing calls `setLocale("tr")`
and there is no picker (DECISIONS-NEEDED, 2026-10-12 phase 5), and `tr` holds
117 of 352 keys, so the release notes now say English, with the switch pending.
## 2026-10-13 — Startup: `%windir%` commands read as unsigned

**Built.** `executable_of` expands `%NAME%` variables before looking for the
program, as Windows does for `REG_EXPAND_SZ` commands. `SecurityHealth`
(`%windir%\system32\SecurityHealthSystray.exe`) now resolves, so its signature
reads "Microsoft Windows" instead of "unsigned", and the resolved path reaches
the OS-owned and protected-path checks. Unknown names, `%%` and an unclosed `%`
stay as written. Pure `expand_env_with` with unit tests, plus one test that
`%windir%\system32\cmd.exe` resolves. Verified live over CDP.

## 2026-10-13 — Counts with digit grouping

**Built.** `formatCount` in `src/lib/format.ts` (system locale, like
`formatDateTime`), used for the last-scan finding count on the dashboard and
cleaner page and for the empty-folders summary: "2530292 found" now reads
"2,530,292 found". Not applied inside `t()`, because `{pid}`, `{id}` and
`{code}` must stay ungrouped. Unit test `format.test.ts`.

## 2026-10-14 — winget manifest for v0.2.0

**Built.** `packaging/winget/` with version, installer and en-US locale manifests
(schema 1.9.0) for `armadavalor.WinKnife` 0.2.0, pointing at the release's NSIS
setup.exe. `winget validate` passes.

**Decisions.** Per-user scope and `ProductCode: WinKnife` match the uninstall key
Tauri's NSIS installer writes (checked on a local 0.1.0 install). WebView2 runtime
declared as a package dependency.

**Deferred.** Local `winget install --manifest` test (needs the admin-only
LocalManifestFiles setting) and the PR to microsoft/winget-pkgs.

## 2026-10-14 — Install smoke workflow and release acceptance

**Built.** `.github/workflows/install-smoke.yml` (manual, with a `version` input):
on a fresh windows-latest runner it downloads the release setup.exe, checks it
against `SHA256SUMS`, installs it silently, launches the app for 15 s and keeps
a screenshot as an artifact, uninstalls it silently, and validates the winget
manifest. v0.2.0 passed every step.

**Decisions.** No `winget install` on CI: winget marks its download with the Mark
of the Web, and SmartScreen holds the unsigned installer at "Windows protected
your PC" (seen in a hang screenshot). The winget-pkgs pipeline ran the full
install test instead and passed (microsoft/winget-pkgs#452789). `winget validate`
on the runner "succeeds with warnings" and exits non-zero because its winget
(1.11) does not know the 1.12 schema header, so the step checks the result line.

**Found.** VirusTotal on the v0.2.0 setup.exe: 3/71, all generic ML detections
(Arctic Wolf, DeepInstinct, SecureAge); Microsoft and the other major engines
clean. Every user who downloads the installer meets SmartScreen until it is
code-signed.

**Deferred.** Code signing; the demo GIF.

## 2026-10-14 — README demo GIF

**Built.** `assets/demo.gif` (GIF 1 of `docs/DEMO-SCRIPT.md`: scan, review,
clean up into quarantine, restore), embedded at the top of the README. Recorded
by the new `demo recording` workflow (`.github/workflows/demo.yml`): it installs
the published v0.2.0 setup.exe after a SHA-256 check, seeds about 1.6 GB of junk
into a fake `C:\Users\Demo` profile, starts the app with the profile variables
pointing there and drives it over CDP with `scripts/demo/record.mjs`. The GIF is
built locally from the frames artifact with ffmpeg (command in DEMO-SCRIPT).

**Decisions.** Recorded on a GitHub runner, not on a real profile, so no real
usernames or paths can reach the frames. WebView2 on the elevated runner ignores
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` and the HKCU policy; the HKLM per-app
`AdditionalBrowserArguments` policy key opens the DevTools port.

**Noticed.** The v0.2.0 cleanup confirm dialog shows the literal title
"Move {count} items to quarantine?" (visible in the GIF); the placeholder is not
filled.

## 2026-10-14 — Cleaner confirm title placeholder

**Fixed.** The Disk Cleaner confirm dialog title rendered the literal
"Move {count} items to quarantine?" because `t("cleaner.confirmTitle")` passed
no params. It now passes `store.selected.size`, the same number the footer and
the dialog body show. Audit of every `{placeholder}` key against its `t()` call
sites found no other bare call (`cleaner.groupCount` has no call site at all).
Regression test `i18n-params.test.ts` fails if a placeholder key is ever called
without params.

## 2026-10-14 — Optional SignPath signing in the release workflow

**Built.** `release.yml` gains three steps after the NSIS build: upload the
unsigned setup.exe, submit it with `signpath/github-action-submit-signing-request@v3`
(waits for manual approval), and overwrite the installer with the signed file
after an Authenticode `Valid` check. Packaging runs afterwards, so the release
asset and `SHA256SUMS.txt` carry the signed installer. `docs/SIGNING.md` holds
the eligibility checklist, application text and the owner's manual steps;
README gets a short "Code signing policy" section.

**Decisions.** All signing steps are gated on the `SIGNPATH_API_TOKEN` secret;
without it the workflow is unchanged. Organization and slugs come from repository
variables. Only the installer is signed.

**Deferred.** The SignPath application itself, MFA, secret and variables (owner
steps), signing `winknife.exe` inside the bundle, the Turkish README section,
and an `actionlint` run.

## 2026-10-15 — Legacy panels launcher (module spec 10, issue #13)

**Built.** A `legacy-panels` module: one page of 22 shortcuts to Windows' own
classic windows (System Properties, Device Manager, Network Connections,
Services, Event Viewer, …) in five groups. The catalogue lives in
`modules/legacy_panels.rs`; the UI asks for it and opens entries by id only.
Every target is an absolute path under `%SystemRoot%\System32`
(`control.exe x.cpl`, `mmc.exe x.msc` or a bare exe). A missing file shows the
entry with a disabled button and "Not on this edition of Windows".
`core/platform/panels.rs` adds the `PanelLauncher` trait: `ShellExecuteExW`,
gated on `real_os_effects_armed()`, with UAC "No" treated as success, plus a
recording fake. 11 unit tests.

**Decisions.** WinKnife writes nothing here: no registry, no files, no network.
The shell (not `CreateProcess`) launches each window so Windows shows its own
UAC prompt.

**Verified.** In `tauri dev`, the page rendered and Device Manager opened.

**Deferred.** The manual check of every button on Windows 11 and of UAC "No"
on an MMC entry. Turkish strings.

## 2026-10-15 — UI polish: themes, grain, copy, consistency

**Built.** Frontend-only pass; no backend or safety-rule changes.

- **Themes.** Three themes as CSS variables in `src/app.css`: Graphite (default,
  no attribute), Forest (deep green) and Mist (light near-grey,
  `color-scheme: light`). `src/lib/stores/theme.svelte.ts` sets
  `data-theme` on `<html>` and persists the choice to localStorage
  (`winknife.theme`). Settings gets an Appearance picker with three swatch
  cards. The treemap canvas switches to a light palette on Mist.
- **Background.** Inline SVG feTurbulence grain plus a soft accent glow in the
  root layout. No network or image assets, so the CSP stays unchanged.
- **Sidebar brand.** Logo, name and the runtime version (`getVersion()`) at the
  top left, linking to the dashboard; the old bottom version line is gone.
- **Copy.** About 95 English and 13 Turkish strings rewritten for a
  non-developer reader. Dashboard cards show a per-module status and action
  instead of the generic "Nothing to scan" text.
- **Consistency.** Shared `btn-primary/secondary/outline/ghost` utilities,
  unified inputs and notices, and a `bg-scrim` dialog overlay. Secondary detail
  (skipped rules, UAC note, startup leftover reason, duplicate-check method,
  unreadable quarantine rows) moved behind `<details>`.

**Deferred.** `quarantine.affectedBatches` is not yet behind `<details>`; the
keep-awake presets have no `aria-pressed`. Turkish coverage is unchanged in
scope (no language picker, K-011).

## 2026-10-15 — Icon set (Lucide)

**Built.** Every small UI icon now comes from one vendored set: 18 Lucide
glyphs (ISC; `power`, `chevron-right`, `info` via Feather, MIT) stored as data
in `src/lib/icons.ts` and drawn by `Icon.svelte` on a 24px grid,
`stroke-width` 1.75, round caps/joins, `currentColor`, so all three themes
tint them. `ModuleIcon` maps manifest icon names onto the set; the hand-drawn
chevrons (FindingList, duplicates), the InfoTip glyph and the EmptyState
fallback use `<Icon>`.

**Decisions.** Vendored as data instead of an npm dependency: no runtime
package, offline, CSP `img-src 'self'` untouched. Legacy panels use
`app-window` (`monitor-cog` was too busy at 20px); empty folders use
`folder-search`. Licence row added to `docs/LISANSLAR.md`.

## 2026-10-17 — QA plan, dirty-machine fixture, Sandbox profile, CDP e2e

**Built.** `docs/QA-PLAN.md`: module × scenario matrix (empty, huge, locked,
access denied, long/unicode paths, links, cancel, undo, restart), each cell an
e2e test, a `cargo test` name or "manual in Sandbox". `scripts/qa/`:
`New-DirtyProfile.ps1` (fake profile with N Temp files, duplicate sets, empty
tree, locked file, deny-ACE dir, junction loop and a junction into
`C:\Windows`, unicode and >MAX_PATH names; refuses the real profile),
`winknife.wsb` + `Start-QASandbox.ps1` + `sandbox-bootstrap.ps1` (read-only
mapped installer, silent install, dirty profile, launch), and `e2e.mjs`, a
CDP suite driving the installed app: cleaner → quarantine restore, empty
folders, duplicates, treemap, keep awake, unlocker (with Stop where there is
one) plus page-load smoke. New `e2e` job in `ci.yml`: PRs to main and manual
dispatch only (`temp_files` input, 100000 = huge Temp).

**Decisions.** Not on push to main: the job really cleans and restores on the
runner. Duplicates Stop is `knownIssue` (xfail) until #38 is fixed. Audit of
long operations that can spin silently recorded in QA-PLAN and filed as
#37–#41; nothing fixed here, the cleaner hang is fixed separately.

## 2026-10-18 — Cleaner at real `%TEMP%` scale

**Problem.** On a machine with ~86 600 files in `%LOCALAPPDATA%\Temp`,
"Scan for junk" walked every pack in about 50 s and then never logged
`scan finished`. The work after the walk ran per finding and in sequence,
showed no progress and could not be cancelled.

**Measured** (`tests/cleaner_scale.rs`, ignored 100 000-file fixture, release,
12 cores):

| phase | before | after |
|---|---|---|
| scan (walk + match + dedupe) | 37 961 ms | 11 108 ms |
| analyze (gates + score) | 36 583 ms | 13 800 ms |
| serialise (81 MB IPC payload) | 220 ms | 280 ms |
| **scan + analyze** | **74.5 s** | **24.9 s** |

**Built.**
- `analyze` judges findings on rayon in chunks of 256. Between chunks it
  checks the cancel token and emits `analyzing` progress
  (`itemsDone/itemsTotal`). Cancelling keeps the judged prefix and marks the
  result `cancelled`. Every finding still passes all three gates (K-006) in
  the same order: path refusal (lexical, then canonical, K-003/K-004), the
  per-file lock probe, then the rule. Nothing is batched, cached or skipped,
  only run in parallel.
- `dedupe_by_path` canonicalises its keys in parallel; the first finding
  still wins, in the same order as before.
- `ProtectedPaths` splits each entry into components once instead of on every
  `reason_for` call. The comparison is unchanged.
- Packs that name the same folder share one walk. The shipped
  `windows-temp`, `crash-dumps-logs` and `installer-leftovers` packs used to
  walk `%TEMP%` three times. Claims stay in pack order, then rule order, so
  "first pack wins" holds, and rules whose root is spelled differently are
  re-anchored onto the walked path.
- UI: the cleaner page shows analyze progress and a Stop button, plus a
  notice when a result was cancelled (EN + TR keys). `FindingList` renders
  200 rows per group with "Show more"; the group checkbox and totals still
  cover every row.
- CI guard: `five_thousand_temp_files_scan_and_analyze_within_budget`
  (not ignored) must finish scan + analyze of 5 000 files in under 20 s in a
  debug build. It takes about 1.5 s locally.

**Decisions.** No gate was weakened, so there is no DECISIONS-NEEDED entry.
The lock probe stays per file; it now just runs in parallel.

**Deferred.** The 81 MB IPC payload is sent in one piece. Streaming or paging
it from the backend is a separate change.

## 2026-10-18 — Stop sends the running scan's id (#38, A9)

**Fixed.** Duplicates Stop cancelled `result.scanId`: null on a first scan,
the previous finished scan on Scan again, so the UI sat on "Stopping…" until
the scan ended by itself. It now takes the id from the scan's own progress
events, as Empty folders and the cleaner do. In both Duplicates and Empty
folders a Stop pressed before the first progress event is sent as soon as that
event names the scan (A9). Frontend only; no backend change.

## 2026-10-18 — Fix: startup disable deadlock and "Not Responding" window

**Fixed.** Turning off any Startup entry stayed on "Working" forever, and the
next click froze the window.

- **Deadlock.** `commands::execute` runs the executor inside `Database::with`.
  `StartupControlAdapter::read`/`disable` then called `Database::with` again,
  on a non-reentrant `std::sync::Mutex`, so they waited forever.
  - `StartupControl` now takes the `&Connection` the executor already holds.
  - The adapter keeps no database handle of its own.
  - Gate 4 (re-read before disable) is unchanged.
  - Regression test: `the_executor_adapter_works_inside_the_database_lock`. It
    drives the adapter under the lock on a thread with a timeout.
- **Frozen window.** 23 sync commands that wait on the database lock, the disk
  or a registry mutex are now `#[tauri::command(async)]`. Before, they ran on
  the window thread, so one stuck lock (above), or a long scan or restore,
  froze the whole UI. The same applied when pressing Stop mid-scan.
  - `tests/command_wiring.rs` now recognises the `(async)` attribute.
- **Kept sync.** These stay on the UI thread on purpose, since they are instant
  or shell launches:
  - `list_modules`;
  - `take_launch_route`;
  - `reveal_in_explorer`;
  - `restore_point_open_settings`;
  - `legacy_panels_open`.
- **e2e.** "duplicates: Stop returns to idle" pressed "Scan again", which only
  resets the page to idle, so Stop never rendered; the fixture scan was also
  too small to catch running. It now scans the big CI folder via `?root=`
  and presses Stop once the walker names a path, as the Empty folders test does.

## 2026-10-18 — Fix: slow clean, Defender CPU on scan, huge list arrows

**Fixed.** A clean of 20 GB junk (84,919 files) took 12 minutes, a scan held
Defender at 85–90% CPU, and the findings list showed giant chevrons.

- **Lock probe** (`core/platform/lock.rs`). The exclusive open asked for read
  access, which Defender scans on access: ~8 ms per file, almost all of the
  per-item cost in the log. It now asks for
  `DELETE | FILE_READ_ATTRIBUTES (| FILE_WRITE_ATTRIBUTES)` (~0.1 ms) and falls
  back to read access only on a denial. Windows' sharing check is our share
  mode against the other handle's access, so detection is unchanged; a
  sharing violation ends the search at once. Tests: a file held open with full
  sharing is still in use; a read-only file is free.
- **Arrows.** f793205 left a `` byte as the chevron's class in
  FindingList and Duplicates, overriding the icon's size. Now `size-4`.
- **Deferred.** The user's polish notes (progress detail, motion, logo and
  background animation, animations-off) are queued in `modules/BACKLOG.md`;
  direct delete is in DECISIONS-NEEDED.

## 2026-10-18 — Commands survive panics; one quarantine operation at a time (#40, #39)

**Built.**
- **Panics answer the WebView (#40).** Every `#[tauri::command]` body now runs
  inside `commands::guarded` (sync) or `commands::guarded_async` (each poll under
  `catch_unwind`). A panic becomes `ApiError { code: "internal" }` with a
  readable message naming the command and the panic text in `detail`, and is
  logged. Before, an async command's task died and its promise never settled,
  so the page spun forever.
- **Busy guard (#39).** New `core/busy.rs`: `OperationLock` +
  `Operation::{Clean, Restore, Purge}`, an RAII guard that releases on drop and
  on unwind. `Quarantine` owns one shared (`Arc`) lock; `execute`,
  `quarantine_restore[_item]` and `quarantine_purge[_item]` take it *before*
  the database lock. A second call gets `CoreError::Busy` (code `busy`,
  "A restore is already running; wait for it to finish") instead of queuing
  behind the DB mutex and running right after.
- Tests: `tests/panic_and_busy.rs` (mock-IPC panics in sync and async commands,
  a source scan that every command body is guarded under its own name, every
  quarantine writer refused while the lock is held and accepted after),
  `busy.rs` unit tests, `busy` added to the stable error codes.

**Decisions.** One lock for all three operations, so a clean-up, restore and
purge are mutually exclusive. The executor takes no lock itself (the command
does), so its three gates (K-006) and the quarantine semantics are unchanged.
The startup retention purge in `lib.rs` runs before any command and stays
unguarded.

**Deferred.** Progress and cancel for execute/restore/purge (rest of #39, #41).


## 2026-10-19 — E2E: Sandbox-only cells moved onto the CI runner

**Built.** `scripts/qa/e2e.mjs` gains the scenarios QA-PLAN listed as manual:
duplicates quarantine/restore (twin untouched), empty folders quarantine/restore
(folder with a file survives), unicode and over-260-character paths round trip
through the cleaner and a restore, restore onto an occupied destination (no
data loss), the double-restore Busy guard (#39/#46: one applied, one refused with
"already running"), and the drawer empty state. New `scripts/qa/e2e-kill.mjs`
runs last in the same CI step: it starts a clean, `taskkill`s WinKnife as soon
as files start moving, relaunches it, and checks that every Temp file is either
at its original path or in the ledger with its quarantined copy present.

**Decisions.** The refusal-is-visible check is a separate test. It found that
the drawer showed nothing for a refused restore; #54 fixed that, and the test is
a plain pass/fail check now. No new fixture sentinels were needed.

**Deferred.** Kill during restore, kill during duplicates/empty-folders moves,
the 100k "Huge" restore, and locked-destination restore with a real OS lock
(the occupied-destination test uses a plain file) stay manual.

## 2026-10-18 — Database lock scope (audit issue #37)

**Built.** The one SQLite connection is no longer held for the length of a
clean, restore, purge, startup scan or treemap drill-down, so lists, settings
and the dashboard stay responsive while those run.

- `core::db::DbAccess` (object-safe; implemented by `Connection` and by
  `Database`, which locks per call) plus the `with_db` helper. The quarantine,
  the ledger, the executor (`ExecEnvironment.db`), `StartupService::scan` and
  `treemap::CacheOracle` take `&dyn DbAccess` and lock only around each
  statement or ledger transaction. File moves run unlocked.
- Ledger guarantees are unchanged: each claim/retire transaction still runs
  under the lock, and the claim → move → undo order is the same. A restore
  claim is now conditional (`AND restored_at IS NULL`), so two restores of one
  item racing in the gap get exactly one move.
- The busy guard from #39 (`Quarantine::begin`) is now what keeps clean,
  restore, purge and startup re-enable (`Operation::Restore`) apart; it used to
  be the database lock that serialised them. A second one is refused at once
  with `CoreError::Busy`. Reads never take it. (The branch's own waiting gate
  was dropped in the rebase onto #46 so there is one lock, not two.)
- Fixes a latent self-deadlock: `execute` held the database lock while the
  startup adapter's `disable` locked it again.
- Tests: `a_long_clean_does_not_hold_the_database` checks that a read from
  another thread finishes while a clean is stuck on a slow file.
- Deferred: `startup_enable` still holds the lock for its single shortcut
  move, which is short. Cancel, progress and busy-guard work stays with #39/#40.

## 2026-10-19 — Quarantine drawer shows refused actions

- The drawer printed `store.error` only when the list fetch failed. A restore
  refused with `CoreError::Busy` set the message but kept the status `ready`,
  and the reload triggered by the other operation's `quarantine:changed`
  cleared it. Found by the e2e QA busy-guard cell (xfail).
- Action failures (restore, purge, item removal, batch items) now go to a
  separate `actionError`, which reloads leave alone; the drawer shows it as an
  alert banner until the next action or until the drawer closes.
- Regression coverage is the e2e cell "the drawer shows the Busy refusal";
  runes stores are not reachable from `node --test`.

## 2026-10-19 — v0.3.0 release prep

- Version 0.3.0 in package.json, package-lock.json, Cargo.toml, Cargo.lock, tauri.conf.json; CHANGELOG `[Unreleased]` cut to `[0.3.0]`.
- README: signing section now says releases are unsigned (K-014); Legacy Panels added to the tool table; stale "preparing v0.2.0" status removed.
- Bulk clean/restore perf (PR #56, closed, probes only): ~1.3 ms/item take and ~1.1 ms/item restore locally (release, on-disk DB); the CI runner is ~8x slower. About half is the per-item ledger commit. Batching commits would widen the crash window (a restore claim marks up to N rows restored before their files move), so it is deferred; `synchronous=NORMAL` was not considered without a decision.

## 2026-10-19 — Cancel and progress for clean and restore (#39)

- Executor takes a `CancellationToken`; between items it checks the token and
  marks every item it did not reach `Skipped` with `CANCELLED_REASON`. An item
  already moving finishes, so nothing is left half-moved.
- `execute` registers the token in `CancelRegistry` under the scan id through a
  drop guard (`CancelTicket`), so `cancel_scan` stops a clean the same way it
  stops a scan, and the registry never keeps a stale entry.
- `Quarantine::restore_batch_controlled` restores with a token and a progress
  callback; `quarantine_restore` emits `quarantine:restore-progress` (throttled
  to 10/s) keyed by batch id and returns `RestoreReport.cancelled`.
- UI: cleaner, duplicates and empty folders share `ExecTracker` +
  `ExecutingPanel` (bar, current path, "n of m", Stop). The quarantine drawer
  shows restore progress and a Stop button on the running batch. A Stop pressed
  before the backend registered the token is resent on the first progress
  event. Stopped items are summed up in one line, not listed one by one.
- Deferred: purge has no cancel yet; the remaining long-operation items stay in
  #41.

## 2026-10-19 — Cold-test kit and launch drafts

- `scripts/coldtest/`: Windows Sandbox kit for the five-person cold first-run test. `Start-ColdTest.ps1` fills in and opens `cold-test.wsb`. Inside the sandbox, `coldtest-bootstrap.ps1` runs a smaller `New-DirtyProfile.ps1`, adds eight old important-looking files, hashes every fixture file into `<P>-manifest.json` (keep or offerable) and opens Edge on the repo. WinKnife is not preinstalled. `Test-ColdTestResult.ps1` classifies each file as unchanged, missing, changed or unreadable; it FAILs on a missing or changed keep file or on any content change.
- The bootstrap only runs as `WDAGUtilityAccount`, so it can't fill a real profile with junk.
- Checked locally on a scratch profile (300 temp files): 2,497 files, 11 keep. Untouched → PASS. One installer moved away → listed as missing offerable; moved back → unchanged. One byte appended to a keep file → FAIL, exit 1.
- Found during the check: PowerShell 5.1 reads the BOM-less UTF-8 manifest as ANSI, so the Unicode-named Temp file showed as missing. Both JSON reads now pass `-Encoding UTF8`. Profile root is `GetFullPath`-normalized like `New-DirtyProfile`, otherwise 8.3 paths missed three keep sentinels.
- Not yet run inside a real Windows Sandbox; the first live session is the end-to-end test.
- `docs/launch/COLD-TEST.md` (protocol, tasks T1–T7, scoring, launch gate), `cold-test-scores.csv`, `POSTS.md` (Show HN, r/software, r/Windows11 drafts checked against v0.3.0, schedule, do-not-claim list). Nothing is posted without the maintainer.

## 2026-10-19 — Cold-test kit: Windows Sandbox dry run

**Built.** A full P9 dry run of the cold-test kit in a real Windows Sandbox
on a Windows 10 host: bootstrap, fixture, download and install of v0.3.0 from
GitHub, a Disk Cleaner run (5,931 items, 99 MB to quarantine) and the
verifier. Verdict PASS: 0 keep files touched, 0 changed, and the report
reached the host results folder.

- **Fix.** The Windows 10 sandbox image has no working Edge (the taskbar pin
  is dead, `microsoft-edge:` has no handler). The bootstrap now installs Edge
  from Microsoft's online installer, refuses it unless Authenticode-valid and
  signed by Microsoft, and launches `msedge.exe` by path. A failed install
  only prints a warning; the fixture still counts as ready.
- **Doc.** `COLD-TEST.md` notes the Edge install, that the sandbox does show
  SmartScreen, and that Defender real-time protection is off inside it, so
  the separate VM session is now for antivirus behaviour.

## 2026-10-19 — Audit #41 leftovers (A4, A7–A10)

- **A4.** `attachProgress` now runs inside the `try` in the cleaner,
  duplicates and empty-folders stores, so a failure resets `step` to idle and
  records the error instead of leaving the scan "running" forever.
- **A7.** `core::content_hash::full_hash_cancellable` checks a cancel callback
  before every chunk and returns `ErrorKind::Interrupted`; duplicates stage 4
  uses it. `full_hash` is unchanged for the executor.
- **A8.** `DuplicateScan.unreadable` counts candidates that failed the partial
  or full read (cancellation excluded) and the duplicates page shows
  `duplicates.unreadable` (English only, like the other duplicates strings).
- **A9.** The empty-folder scan command emits an initial progress event with
  the scan id before walking; the store already forwards a pending Stop on the
  first event.
- **A10.** The treemap cache save was not atomic (separate DELETE and INSERT
  statements). It is now one transaction; a failed save keeps the old cache.
- Deferred: the cleaner walk on `spawn_blocking` (second half of A4).

## 2026-10-19 — Polish item 5: progress bars that move

- Running bars no longer look frozen. Unknown totals get a sliding segment
  (was `animate-pulse`); known totals on running work get a drifting stripe.
  Current path and count were already on the clean and restore panels (#39).
- The rule sits in `src/lib/progress/mode.ts` (tested): finished bars and
  gauges (`active={false}`, used by the treemap disk capacity) stay still.
- Motion is off under `prefers-reduced-motion` and when `<html>` carries
  `data-animations="off"`, the hook Polish item 8's switch will set. Without
  motion the indeterminate bar fills at half opacity instead of a stuck third.

## 2026-10-19 — Polish item 8: animations off switch

**Built.** An Animations toggle under Settings › Appearance, on by default and
stored locally like the theme (`winknife.animations`). `src/lib/motion.ts`
holds the rule (on only when the toggle is on and the system does not ask for
reduced motion; tested), `stores/motion.svelte.ts` watches
`prefers-reduced-motion` live and sets `data-animations="off"` on <html>. One
CSS block stills every keyframe animation and transition under that attribute;
progress bars keep their own still look from item 5. The row says when Windows'
reduced-motion setting is what keeps animations off.

**Deferred.** "Off in a performance mode": no such mode exists yet
(DECISIONS-NEEDED). Items 6–7 (logo, background light) must key off the same
attribute.

## 2026-10-20 — Polish 6–7: knife logo animation and background light

**Built.** `KnifeLogo.svelte`, an SVG redraw of `assets/logo.png` whose blade
rotates around the hinge: one fold-and-open on sidebar-logo hover, a loop while
modules load, and a small looping knife in front of every loading line
(`LoadingNote.svelte`, six places). Background light: three variants (Aurora,
Waves, Spotlight) picked under Settings › Appearance and stored locally
(`winknife.glow`), plus a cursor light that eases toward the mouse through
registered `--glow-x/--glow-y`. `src/lib/glow.ts` holds variant parsing and the
pointer-to-percent maths (tested); the store throttles pointer updates to one
per frame and ignores them when animations are off. Glow opacity raised per
theme (0.5 / 0.55 / 0.7). Layers only animate transform and opacity.

**Decisions.** With animations off the knife stays open and still and the
cursor light parks in the middle; the global `data-animations="off"` rule
already stops the keyframes. The PNG stays the canonical logo source; the SVG
is a drawing of it (KANONIK).

## 2026-10-20 — Background light reaches the whole window

**Built.** Feedback on Polish 7: the light stayed at the top. Aurora now has
one blob from the top-left and one from the bottom-right that drift past each
other through the middle. Waves adds a fainter band layer that rises from the
bottom and rolls the other way. Spotlight adds a dimmer floor pool that
breathes out of step with the top light. Still transform and opacity only;
`data-animations="off"` stops everything as before.

## 2026-10-20 — QA/QC pass (Polish 9)

**Built.** `docs/QA-CHECKLIST.md`: 35 cases over clean / quarantine / restore /
purge and every scanning module's Stop and rescan, each with the expected
behaviour, the test that checks it and a status (OK, BUG → fixed,
BUG → decision, UNTESTED, n/a).

**Fixed (failing test first).**
- Duplicates e2e flake `duplicates: Stop cancels the scan`. The UI defers a
  Stop pressed before any progress event until an event names the scan, but the
  backend's first duplicates event only came with the first qualifying file. On
  a sparse or slow start the cancel went out late or never. The scan now emits
  one event naming each root before walking it.
- A Stop accepted after the pipeline's last cancel check was lost:
  `cancel_scan` said true, yet the full result came back unstopped.
  `CancelRegistry::finish` now reports a tripped token, and the command turns
  the result into `DuplicateScan::stopped()`.
- The stored duplicates `ScanResult.cancelled` was always `false`.
- Treemap: a Stop before the first event was a no-op (now deferred like
  duplicates, with the button disabled while stopping); the old scan id stayed
  across rescans; late events revived a finished id.
- New UI regression `scripts/scan-stop-regression.mjs`, run by `test:ui`.

**Decisions.** A crash between the restore claim and the move orphans the item
(hidden, never purged). It is a destructive path, so it is escalated in
DECISIONS-NEEDED with a proposed startup repair, not changed here.

**Deferred.** The UNTESTED rows in the checklist: lock at restore time, long
paths across volumes, e2e Stop for execute and restore, purge partial failure,
and UI frame time on large trees.

## 2026-10-20 — QA/QC: untested quarantine cases, A9 repair

**Built.** Tests for QA A5 (restore onto a file where the parent was), A8
(locked item in a batch restore), A13 (paths over MAX_PATH across volumes,
purge and the lock probe) and A17 (partial purge). A9 fix: at startup
`Quarantine::recover_interrupted_restores` clears `restored_at` on claimed rows
whose quarantine copy still exists inside the root, before retention, so an
item from a killed restore is held again.

**Decisions.** User approved the A9 repair (K-017). Rows whose path is outside
the quarantine root are not trusted. Leftovers of a finished cross-volume
restore also come back as held; nothing is deleted.

**Deferred.** A18 (needs a real second volume), A19 (UI frame time), A14/A15
UI Stop end to end.

## 2026-10-23 — QA: UI end-to-end for Stop and partial failure (A14, A15, A19)

**Built.** Three cases in `scripts/qa/e2e.mjs`, run by the CI e2e job:
Stop pressed mid-clean (moved items held, rest left, run shown as stopped, UI
idle), Stop pressed mid-batch-restore (restored stay restored, rest held, UI
idle, restore can be run again), and two for a locked file. One checks a file
locked before the scan: it is listed under "Left alone" with its reason and has
no tick. The other locks a ticked temp file from the harness after the scan
(PowerShell, `FileShare.None`), cleans, and checks that the done view lists it
as "in use by a running program", it stays in place, the ledger omits it, the
rest moved and the freed size matches the ledger. Stop is
pressed from an in-page MutationObserver once the run's progress line shows
"N of M" with 0 < N < M; each case restores all held batches in a `finally`; the 5000-file fixture gives the window. No production hook added.

**Fixed after the first CI runs.** The e2e's `ledgerItems()` counted rows
already restored: `quarantine_batch_items` returns every row of a part-restored
batch, so A15 saw "6506 of 6506" after a Stop that had in fact restored some
(test measurement only; the backend commits per item and was not changed). It
now keeps only rows without `restoredAt`. `truncatePath` keeps a short file name
whole instead of cutting its first letters (display only; a path with no `\`
is cut as before).

**Deferred.** Not run locally (a local run really cleans and needs a QA-identifier
build); the CI e2e job is the verifier. A18 still needs a real second volume.

## 2026-10-24 — Hide tools from the sidebar (issue #8)

**Built.** A "Tools in the sidebar" section in Settings lists every module with
a checkbox; unticking one takes it out of the sidebar.

- **Pure helpers** (`src/lib/hiddenModules.ts` + test): parse the stored list
  (anything but a JSON array of strings means none, duplicates dropped), toggle
  an id immutably, filter modules keeping registry order.
- **Store** (`stores/hiddenModules.svelte.ts`): localStorage key
  `winknife.hiddenModules`, read on mount like the theme and glow; blocked
  storage keeps the choice for the session only.
- **Decision.** Hiding is visual only. The dashboard still lists every tool and
  the route and `--module <id>` still open it, so nothing becomes unreachable
  and no module settings or Rust state change.
- **Edge.** With every tool hidden the sidebar says so and points to Settings
  instead of "No tools are available yet."

## 2026-10-24 — Startup page shows the rating and its reason (#4)

**Built.** Every entry Rust offers to turn off now shows the rating it was
given (green "Safe to turn off", yellow "Review before turning off") in
place of the grey known-app label, and a "Why this rating?" fold with the
recommendation's `reason`. Entries without a rating (already off, owned by
Windows) keep the known-app label. `StartupStore.ratingOf(entry)` exposes the
recommendation the store already paired with each entry.

**Decisions.** The protected-program reason no longer quotes the cleaner's
gate text ("not cleanable … use the app's own uninstaller"): it names the
place from the protected list's own entry (`ProtectedPaths::reason_for`,
cut before its advice), and keeps the gate's words only for a refusal the
list did not make. Closes DECISIONS-NEEDED 2026-10-03 notes (2) and (3).

**Tests.** `startup-regression.mjs` asserts the store hands back the
rating per entry and `null` for an unrated one; the Rust test for a
protected program asserts the new wording and that none of the cleaner's
words leak in.

## 2026-10-24 — C3: duplicate folders

**Built.** The duplicates scan now also groups folders whose file sets are
identical: same relative paths, same content. The group key is a blake3 hash
of the sorted (relative path, content hash) pairs (`modules/duplicates/folders.rs`).
The scan stays read-only. A new "Duplicate folders" section on the duplicates
screen shows each group with its size and wasted bytes. Every member row
carries a reason.

- **Keep rule per group.** Oldest (the default), newest, or a preferred folder,
  set through `set_folder_keep_rule`. Changing it swaps the group's
  recommendations in the `ScanStore` in place (`ScanStore::replace`), so
  `execute` only sees the current plan.
- **Selection.** Files in non-kept copies start selected for Quarantine. File-set
  bulk actions keep the folder selections. Nothing deletes directly; the
  existing quarantine executor runs it.
- **Never every copy.** `pick_keeper` always returns exactly one index, and the
  keeper's files never get a Quarantine action. A final guard
  (`keep_at_least_one`) works per relative file: if every copy of a file would
  be quarantined, it turns all of them into info-only Do-not-delete. Both are
  tested
  (`exactly_one_folder_is_kept_…`, `the_guard_never_lets_every_copy_be_quarantined`).
  The folder keeper and the file-set keeper can differ. Folder defaults plus
  "Select every non-keeper" can therefore tick every copy in the UI. Executor
  gate 4 still refuses the last copy: units run in order, and by then its
  recorded keeper is gone. `copies_that_name_each_other_as_keeper_never_all_go`
  tests this.
- **Protected paths.** A protected member is shown but never selected or rated
  Safe. A folder with protected contents is never grouped.
- **Ignore this group.** Ignored group ids persist in the existing settings
  table under `duplicates.ignoredFolderGroups`. No new migration (C1 owns 005).
  `clear_ignored_folder_groups` resets the list. There is no UI for it yet.

**Known limits (deferred).**
- The folder pass hashes files a second time.
- Hard links are not collapsed.
- Empty directories stay behind after quarantine.
- Hidden/system files and empty subfolders are left out of the comparison.
- Filename case counts.
- The ignore key is content-based, so a changed folder shows up again.

## 2026-10-24 — Reversible tweak engine (C1)

**Built.** A `tweaks` module with a short in-code catalog of three per-user
Explorer settings (show file extensions, show hidden files, open Explorer to
This PC). Applying is a new executor action, `Action::ApplyTweak`. Before it
writes, it records the real prior value, or "absent", in the new
`tweak_undo` table (migration 005). After writing it reads the value back,
and a mismatch rolls the write back. The Tweaks page shows each setting's
registry path, the value now, the value after, whether a restart is needed
and the documented source, all before anything changes. Apply goes through a
confirm dialog; applied settings get an Undo button (`tweaks_undo`).

**Decisions.** K-018: HKCU only. The catalog sits behind an in-code fence
that refuses Windows Update, search index, Start menu and Defender keys, and
no "not undoable" tweak ships. Undo puts the recorded value back only while
the live value is still the one we wrote. If the prior value was "absent",
undo deletes only the value we wrote. The executor re-checks the live value
before writing, as it does for every other action.

**Tests.** Rust unit tests cover these cases:
- apply then undo restores the original bytes, and an absent value is absent again
- the fence refuses without writing
- a changed or already-set value is refused
- a non-number value is left alone
- undo is refused after an outside change
- a failed write leaves no undo record
- a write that does not read back keeps the old value
- re-applying supersedes the older record

**Deferred.** No store regression script yet, because the store mirrors the
startup one. The module keeps the generic "settings" icon. C2, C7 and C10
get hook notes only, in `docs/phases/PHASE-6-tweaks.md`.

## 2026-10-24 — Hosts file editor (C4, K-019)

**Built.** A `hosts` sidebar module. It parses `%SystemRoot%\System32\drivers\etc\hosts`
into entries, disabled entries, comments and invalid lines (flagged with
the line and reason), and edits a draft: add, remove, turn on/off, or raw
text. Every draft is validated and diffed against the file on disk; Save
asks for confirmation, takes a timestamped backup in app data, then writes
through the UAC helper (`--write-hosts`). Encoding (UTF-8/BOM/UTF-16) and
line endings are kept. Any backup restores in one click (after a backup of
the current file). `ipconfig /flushdns` runs only when its button is
clicked, and the page shows the exact command.

**Decisions.** The write exception covers only the hosts file and our own
same-folder `hosts.winknife.tmp` (`HostsFence`, canonical path equality
from `%SystemRoot%`, checked before every write; any other path is refused
and tested). The write is a temp + `ReplaceFileW` swap with read-back
verification; the helper refuses empty or unvalidated bytes (hash check)
and reports a file changed since the backup instead of overwriting it.
Backups are never deleted automatically. A declined UAC prompt is a plain
"nothing changed" outcome.

**Tests.** Parser, diff, encoding/line-ending round trips, the fence
(refuses other paths, including `C:\Windows\…` siblings), save/restore/no-change
and changed-underneath paths against a fake writer in a tempdir; the real
hosts file is never touched by tests.

## 2026-10-24 — C9: provenance and verification

**Built.** README "Verify this download" (official URLs, `Get-FileHash`,
`gh attestation verify`), `docs/ANTIVIRUS-FALSE-POSITIVE.md`, an About block
in Settings ("What WinKnife never does", official URL as text, no network
call), and `actions/attest-build-provenance` in `release.yml` for the
installer, ZIP and `SHA256SUMS.txt`.

- `SHA256SUMS.txt` was already produced by `scripts/package-release.ps1`; no
  change needed there.
- Release workflow gained `id-token: write` and `attestations: write`. Triggers
  untouched; no tag pushed.
- README says winget is submitted but maybe not live and that Scoop does not
  exist, rather than listing install commands that fail.
- Deferred: the attestation step is untested until the next tag build.

## 2026-10-24 — C2: context-menu cleaner

**Built.** `modules/context_menu.rs` + `/context-menu` page. The scan reads
`shell` verbs and `shellex\ContextMenuHandlers` for `*`, AllFilesystemObjects,
Directory, `Directory\Background`, Drive and Folder, in HKCU and HKLM
`Software\Classes`. Each entry's reason names the owner: verb command or
handler CLSID → `InprocServer32` → file → Authenticode signer. No signer =
"owner unknown", needs review; nothing is ever pre-ticked.

- Disable rides the C1 pipeline (K-018): `Action::ApplyTweak` with tweak id
  `context-menu:<entry>`; a small `TweakRouter` sends prefixed ids to this
  module, so the executor and `tweaks.rs` are untouched. A verb gets
  `LegacyDisable` (HKCU); a handler gets its CLSID under HKCU
  `Shell Extensions\Blocked`. The old value (or "absent") goes to
  `tweak_undo` before the write, read back after; Re-enable restores it and
  refuses if the value changed since.
- HKLM entries are listed read-only ("needs admin; not changed"); no code path
  writes HKLM (tested with a recording fake registry). `Registry` gained
  `subkeys`.
- Tests: owner classification, verb and handler disable/undo round trips,
  HKLM never written, shared-CLSID case, undo-after-change refusal, router.
- Deferred: whether Explorer (and the Windows 11 new menu) honours the HKCU
  block needs a manual check; see DECISIONS-NEEDED.

## 2026-10-24 — C7: drift check and Windows Update status

**Built.** The Tweaks scan now compares each live value with the value C1
applied. A value changed since (by Windows, policy or another tool) is shown
as drifted with a reason and a manual "Re-apply" button, which runs the
ordinary C1 apply path (fence + undo row). A new read-only "Is something
blocking Windows Update?" section (`tweaks/update_block.rs`) reads the HKLM
policy values, the `wuauserv` start type, a `WUServer` override and an active
pause; it only calls `Registry::value`, never writes.

- Fence (`fence_refusal` / `FENCED_FRAGMENTS`) unchanged.
- Values that cannot be read are listed as errors, so "nothing found" is
  never claimed for a key the check could not see.

## 2026-10-24 — C10: settings profile export/import

**Built.** Settings → "Settings profile": export writes a versioned JSON
(`version`, tweak ids, cleaner packs, duplicate settings, quarantine
retention) to a file the user picks; import reads it, shows a diff (per
tweak from → to, per setting) and applies nothing until confirmed. Tweaks go
through the ordinary scan + `execute` path, settings through the ordinary
settings save.

- "Selected tweaks" in the brief was interpreted as **applied tweaks**: the
  export lists the tweaks currently applied; there is no separate selection.
- Import boundary (`tweaks/profile.rs`): 64 KiB cap before parsing,
  `deny_unknown_fields`, strict types, version 1 only, every tweak id must be
  known and inside the fence; unknown pack ids and invalid retention are
  refused by the settings merge. Test: a hand-crafted profile naming a fenced
  or unknown tweak is refused with no registry writes and no undo rows.

## 2026-10-25 — Admin elevation on request (K-021)

**Built.** WinKnife can tell whether it runs elevated and restart itself as
administrator on a click; actions that fail for lack of rights say so instead
of showing a raw "access denied".

- `core/platform/elevation.rs`: `is_elevated` (TokenElevation),
  `relaunch_elevated` (ShellExecuteExW `runas`, same exe; UAC "No" = 1223 →
  `Declined`), `relaunch_parameters` (keeps `--module` only for a plain id,
  `--dry-run`, `--after-elevation <pid>`), `wait_for_exit`. The elevated copy
  waits for the old pid before the single-instance plugin runs, so it is not
  turned away. Not armed in tests (`elevation_blocked`).
- Commands `elevation_status`, `restart_as_admin` (exits on `Started`).
- `CoreError::NeedsAdmin` / code `needs_admin`; `Io` PermissionDenied, registry
  `ERROR_ACCESS_DENIED` and Task Scheduler `E_ACCESSDENIED` map to it.
  `ExecOutcome::Failed` gains `needsAdmin`.
- UI: Standard/Administrator badge + "Restart as admin" in the sidebar; a
  banner when any command or exec item comes back `needs_admin`; UAC declined
  keeps running with a notice. Failed items show a fixed "needs administrator
  rights" note.
- Deferred: "Always start as admin" rejected (DECISIONS-NEEDED); upfront HKLM
  hint on the startup page left to the list-QoL work.
## 2026-10-25 — Quarantine purge feedback and drawer QoL

**Fixed.** "Remove for good now" looked broken. It runs only the retention
purge and the size cap, so items still inside their keeping period stay, and
the drawer said nothing when nothing was due. On top of that,
`items_in_batch` also listed restored rows, so a batch kept showing items that
were already gone from quarantine.

- `items_in_batch` filters `restored_at IS NULL`. Test:
  `a_restored_item_is_not_listed_in_its_batch`.
- `PurgeReport.failureReasons` gives each failed id a reason: `locked`,
  `accessDenied`, `outsideQuarantine` or `other`, mapped from the io error.
  Tests: `a_purge_blocked_by_an_open_file_reports_it_as_locked`, and the climb
  test now asserts `outsideQuarantine`.
- Drawer:
  - the purge asks for confirmation;
  - the button shows "Removing…" while it runs;
  - a status line always follows: either what was removed, the reasons for any
    failures (with an admin hint for access denied), or when the oldest group
    becomes due.
  - Expanded lists are fetched again, so purged rows leave at once and failed
    ones stay with a reason badge.
- QoL:
  - a path filter;
  - per-item checkboxes with select-all-shown;
  - "Restore selected", which reuses `quarantine_restore_item` one item at a
    time.
  - No new deletion path: the purge scope is unchanged.
- The view logic lives in `src/lib/stores/quarantineView.ts`, with node tests.

**Remove for good, by hand (K-020).** The user decided the deferred point:
- `Quarantine::purge_items(ids, confirmed)` replaces `purge_item`. It removes
  any held item through the same `destroy()` root guards, and returns a
  `PurgeReport` with per-item failure reasons. Without `confirmed` it returns
  `purge_not_confirmed` and touches nothing. The `quarantine_item_not_marked`
  check is gone.
- Command `quarantine_purge_items(itemIds, confirmed)`; the old
  `quarantine_purge_item` is removed. The probes in `command_wiring` and
  `panic_and_busy` were updated.
- Drawer: "Remove {count} for good" for a selection, "Remove for good" per
  batch, and "Remove" on every item. One dialog names the item count and total
  size and says it cannot be undone. Items that are locked or refused stay
  listed with a reason badge, and the summary line counts them.
- Test `removing_items_for_good_needs_confirmation` replaces the old
  "unmarked item is refused" test.
- "Remove for good now" (expired only) already says when nothing is due and
  until when the oldest group is kept.
---

## 2026-10-25 — List QoL: shared filter / select-visible toolbar

**Built.** One `ListToolbar.svelte` + pure `listTools.ts` (node:test covered) used by
Cleaner, Duplicates (files + folder groups), Empty folders and Startup: case-insensitive
filter, sort (Duplicates, size/name), select all shown / none / invert. Cleaner groups
collapse (persisted in localStorage); a reason shared by a whole group is shown once.

**Decisions.** Bulk select only touches visible rows that are already selectable: red,
recycle-bin, keepers, protected folder members and non-quarantine rows are never ticked.
Filter and sort are view-only. "Select none" now clears only visible rows.

**Deferred.** File unlocker (few rows) untouched; no collapse persistence on Duplicates
(set ids are per scan).

**Review follow-up.** A filter that hides still-ticked rows now says so ("+N selected, hidden by the
filter", `hiddenSelectedCount`, unit-tested); selection itself is unchanged. Screenshots of Cleaner
and Duplicates (toolbar, middle-ellipsis paths with full path in the tooltip) checked in the dev app.

## 2026-10-25 — Tweaks catalogue: 27 documented HKCU settings, grouped page

**Built.** The tweak catalogue grew from the original few to 27 entries, each a
single `REG_DWORD` under HKCU with a human `reason`, a Microsoft docs or
settings source URL and an "applies to" note (Windows 10 and 11, or Windows 11
only). Categories: File Explorer (7), Taskbar & Start (7), Privacy (5),
Windows & input (4), Appearance (3). `Tweak` gained `category` and
`applies_to`; `TweakState` carries both to the UI.

The page now groups tweaks under section headers, has a search box (name,
reason, path), shows a state badge per row (Windows default / Applied by
WinKnife / Set another way) and the "applies to" line inside the collapsible
"Why change this?" block next to the source. After an Explorer-restart tweak is
applied the page shows a notice to sign out or restart Explorer.

**Decisions.** No "Restart Explorer" button: killing explorer.exe closes the
user's open folder windows, so a notice is the safe option. No engine or fence
change: the engine's rules (HKCU only, prior value saved first, read-back,
exact undo) apply unchanged.

**Tests.** `catalogue_is_well_formed` (unique ids and paths, HKCU, reason and
https source present, valid targets, undo class matches restart class) and
`every_shipped_tweak_round_trips_on_a_test_registry` (apply, undo, value
restored, no open undo rows; run once from an absent value and once from the
opposite value). Live on this machine through the dev app: shake to minimize,
Snap Assist and clock seconds applied (values 1, 0, 1) and undone (all back to
absent).

**Deferred.** See DECISIONS-NEEDED, "Open items raised by the tweaks catalogue".

## 2026-10-25 — UI language: English + Türkçe (K-022)

**Built.** Settings → Language (English / Türkçe). Choice kept in localStorage `winknife.locale`; with no choice the default is Turkish when `navigator.language` starts with `tr`, else English. Switching applies live: a reactive `locale` store, `<html lang>` updated, and the layout re-renders under `{#key locale.current}` (`t()` itself stays a plain lookup, no i18n library).
- `i18n.ts`: full TR dictionary for every key (terms: Karantina, Temizleyici, Yinelenenler, Boş klasörler, Başlangıç, İnce ayarlar, Disk haritası, Kapatma zamanlayıcısı, Bağlam menüsü, Hosts düzenleyici; "sen" register). Module names/descriptions for the sidebar moved from hardcoded to `module.<id>.*` keys.
- Store-side hardcoded errors/notes (`execNotes`, startup, context-menu, tweaks, elevation) now go through `t()`.
- `format.ts`: bytes, counts, remaining time, dates and relative days follow the active language.
- `i18n-parity.test.ts`: EN/TR key parity and `{placeholder}` parity; `localePref.test.ts`, `format.test.ts` extended.

**Deferred.** Rust-generated reasons/errors stay English (see DECISIONS-NEEDED, 2026-10-25 K-022).

## 2026-10-25 — v0.4.0 release prep

- Version 0.4.0 in package.json, package-lock.json, Cargo.toml, Cargo.lock, tauri.conf.json and the demo/install-smoke workflow defaults; CHANGELOG `[Unreleased]` cut to `[0.4.0]` and filled in for everything merged since v0.3.0 (#61–#92).
- README: thirteen tools, Tweaks / Context menu / Hosts File rows, EN+TR noted. First release with the build attestation step (C9).

## 2026-10-25 — Network module

**Built.** `network` module (C14, K-023, `docs/phases/PHASE-7-network.md`): read-only list of listening TCP (v4/v6), bound UDP and established TCP connections with PID, process name, exe path, reach (all interfaces / this PC only / one interface) and a plain-language reason per row. New platform seam `core/platform/network.rs` (`GetExtendedTcpTable` / `GetExtendedUdpTable`), command `network_list`, `src/routes/network` page with filter, kind filter, refresh and copy-row. The only action is "Flush DNS cache", reusing `hosts_flush_dns` behind a confirm. EN + TR strings for everything.

**Decisions.** Read-only; no process kill, winsock/IP reset, firewall change or outbound lookup this round. The snapshot is taken on open and on Refresh, not live.

**Deferred.** See DECISIONS-NEEDED "network module".

---

## 2026-10-25 — Phase 7: accessibility pass (C8)

**Built.** `focusTrap` action (focus on open, Esc, focus return, optional Tab trap) for ConfirmDialog and the quarantine drawer; skip link target is focusable; scan status lines are live regions; contrast tokens fixed for AA in graphite, forest and mist; sidebar scrolls on short windows.
**Decisions.** No new strings, no backend change. See `docs/phases/PHASE-7-a11y.md`.
**Deferred.** Graphite `ink-muted` on `accent-soft` (3.88:1, hover/active only).

## 2026-10-25 — Phase 7: repair kit (C6)

**Built.** New `repair` module: SFC and DISM (CheckHealth, ScanHealth, RestoreHealth, AnalyzeComponentStore) on click, exact command shown, live output over `repair:output`, one-line verdict, full log under `log_dir` with "Open log". `SystemRepairRunner` (CREATE_NO_WINDOW, UTF-16/OEM decoding, cancel kills the child) or `FakeRepairRunner` when real OS effects are not armed.
**Decisions.** K-025. RestoreHealth and sfc confirm first; unelevated runs return `needs_admin`. See `docs/phases/PHASE-7-repair.md`.
**Deferred.** StartComponentCleanup; verdict parsing is English-only with exit-code fallback; repair logs are not pruned (DECISIONS-NEEDED).

---

## 2026-10-25 — C19: installed apps

**Built.** Read-only `apps` module: Win32 programs (three Uninstall hives) and Store packages, with search/filter/sort, lazy Authenticode signer check, "Open in Settings" and "Show install folder". See `docs/phases/PHASE-7-apps.md`.

- **Decision.** Store packages come from the HKCU `AppModel\Repository\Packages` registry keys rather than PowerShell `Get-AppxPackage`: no process spawn, no WinRT, same read-only guarantee. Trade-off: names are often unresolved `@{...}`, so those show the package family.
- **Decision.** No uninstall action by design; the only way out is Windows Settings.
- **Deferred.** Per-user Win32 installs outside the Uninstall keys, and Store display names/logos (needs the package manifest).

---

## 2026-10-25 — Environment variables (C15, K-024)

**Built.** New `env-vars` module and page (`docs/phases/PHASE-7-env-vars.md`).
- Lists user variables (`HKCU\Environment`) and system variables (HKLM Session Manager, read-only, never written). A user value that is not `REG_SZ`/`REG_EXPAND_SZ` is read-only with a reason.
- PATH-like values (containing `;`) split into entries, each flagged with a reason: empty, duplicate (case-insensitive, trailing `\` ignored), missing directory (after `%VAR%` expansion), unknown `%VAR%`. The editor edits them as rows (reorder, add, remove).
- Every change is previewed first (before/after kind + per-entry diff), then written through the C1 `tweak_undo` table with id `env:<NAME>`: prior value or "absent" recorded, exact kind written, read back (mismatch restores the prior), delete stored as the `REG_NONE` sentinel. Undo refuses after an outside change. `WM_SETTINGCHANGE "Environment"` broadcast via `SendMessageTimeoutW(SMTO_ABORTIFHUNG, 1000 ms)`, only when real OS effects are armed.
- Refused before any registry call: system scope, empty name / `=` in name / over-long name or value, stale `expected`, add over an existing name.
- Tests against `MapRegistry` + in-memory DB: split/join round-trip, all four flags, validation, create/modify/delete → undo (kind kept), HKLM refusal, stale and outside-change refusals.

**Deferred.** See DECISIONS-NEEDED, "Open items raised by the environment-variable editor".

## 2026-10-25 — C7 follow-up: type-changed drift test

C7 (drift check, manual re-apply through `apply`) already shipped in 5e4bf0e; this adds the one missing case from the brief, a value whose registry type changed (`REG_DWORD` to `REG_SZ`), pinned as drift with a reason. No behaviour change.

## 2026-10-25 — Cancel and timeouts for long operations (#41)

- **A7, A9, A10** were already fixed by #62 (`full_hash_cancellable`, the
  empty-folders Stop before the first progress event, the atomic treemap cache
  save). Each was checked against the code; no change here.
- **A8.** Folder duplicates now count members that could not be hashed in
  full (locked, unreadable) into `unreadable`, so the UI warning shows them
  instead of the folder quietly losing a file. Test holds a no-share handle.
- **A11.** `RestorePointService::create` resets its busy flag through a drop
  guard, so a panic mid-request no longer leaves the service stuck at "busy".
  The bounded wait was already there (`HELPER_TIMEOUT`, 5 minutes).
- **A12/A13.** `TimedRestartManager` wraps the real Restart Manager: the
  "who uses this file" query gives up after `QUERY_TIMEOUT` (30 s) and the
  polite close after `CLOSE_TIMEOUT` (60 s), with a new `RmError::TimedOut`
  that the unlocker already shows as a failure message. The close stays the
  same polite request; nothing is forced. Shortcut: a timed-out call keeps its
  worker thread until Windows answers.

## 2026-10-26 — Big-file archiver (C16)

- New `archiver` module (`modules/archiver.rs`, sidebar "Big-File Archiver").
  Read-only scan lists files of at least `MIN_SIZE_BYTES` (1 GiB) not used
  for `MIN_AGE_DAYS` (365) days; age is the newer of last write and last
  access. Protected-path files are counted (`skippedProtected`) and, if
  listed, rated `doNotDelete` and cannot be selected. Every finding has a reason.
- `archive_files` moves a selection to a user-chosen folder: copy with
  `create_new` (never over an existing file), verify size + `full_hash`
  against the source, then send the original to Quarantine.
  A failed copy or verify leaves the original untouched; the module removes
  nothing, so a half-made copy stays in the destination and the report names it. Each archived file appends a line to
  `WinKnife-archive-manifest.jsonl` in the destination (original path,
  archived path, size, hash, quarantine item id, batch id).
- The destination is validated as external input: must exist, be a folder,
  not be protected, and not lie inside the scanned root.
- Commands `start_archiver_scan`, `archive_files` with wiring probes; EN + TR
  strings; route `/archiver`.
- Decisions: K-026. Deferred: cancelling a running move, restore from the
  manifest in one click (restore goes through Quarantine today).

## 2026-10-26 — C18: similar-image duplicates

**Built.** A "Similar images" mode in the Duplicates tool. JPEG, PNG, WebP and
GIF are decoded with the `image` crate (0.25, only those format features) and
hashed with a 9x8 dHash; images within `SIMILAR_MAX_DISTANCE = 8` bits are
grouped with union-find. Scan is read-only, cancellable, reports progress and
decodes on a bounded rayon pool (4 threads) so memory stays flat.

- **Safety.** Nothing is preselected. Each group keeps one image (largest
  pixels, then largest file); the keeper is never selectable and the user can
  move it with "Keep this one" (`set_similar_image_keeper`). Protected members
  are DoNotDelete/InfoOnly. Removal goes through quarantine and the executor;
  `Justification::SimilarImage` makes gate 4 re-hash the file and refuse if
  its content changed or the keeper is gone.
- **Commands.** `start_similar_image_scan`, `set_similar_image_keeper`, both
  covered by `command_wiring` probes. EN + TR strings.
- **Review fixes.** Online-only cloud files (OFFLINE / RECALL_ON_OPEN /
  RECALL_ON_DATA_ACCESS) are skipped from their attributes without being
  opened, so the scan never downloads OneDrive placeholders; the count is
  shown as `cloudOnly`. Each file's decode runs under `catch_unwind`, so a
  decoder panic drops one file. The Duplicates minimum file size applies here
  too.
- **Decisions.** K-029. HEIC/JXL, thumbnails and a threshold setting deferred
  (DECISIONS-NEEDED).
