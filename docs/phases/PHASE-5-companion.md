# Phase 5 — "everyday companion" (v0.2.0)

Prereq: PR #1 merged (2026-10-12). Read ARCHITECTURE §2, §6, §7 and the module specs named below. Work lands on branch `feat/v0.2.0` in the order listed; each task is its own commit (or small series) and must pass the definition of done on its own.

## Tasks

0. **Issue #21 follow-ups (decided 2026-10-12, user).**
   - Startup stash "both kept" arm → option **(b)**: the entry shows as *disabled, but its shortcut is still in the Startup folder* (new status/label + reason, EN+TR). Re-enable: when the original `.lnk` at that name has the **same contents** (byte-equal, compared through the existing content hash) as the stored copy, the stored copy goes to **quarantine** (never `fs::remove_*`) and the entry reads as enabled. Different contents → keep today's `Occupied` refusal and say so. Unit tests for both arms.
   - Encrypted (EFS) kept items outside the size cap → **keep current behavior**; the drawer/Settings warning is enough. Close the item in DECISIONS-NEEDED.
   - Dedicated error code for the "not marked" refusal in `quarantine_purge_item`; UI maps it explicitly.
   - Setup errors after `db::open` must reach the log file (keep the log guard alive until the error is logged).
   - Downgrade mitigation is **not** done (downgrades unsupported); record as closed-won't-fix.
   Check off #21 in the PR description.
1. **Single instance + `--module <id>`** (issues #6, #7, closes the risk in #3).
   - `tauri-plugin-single-instance`. A second launch never builds a second app: it forwards argv to the running one, which shows, un-minimizes and focuses its main window (also when hidden in the tray) and then exits.
   - `--module <id>` (also `--module=<id>`) opens that module's route, on first launch and when forwarded. Unknown id → log at warn, open the dashboard, no error dialog. Parse it in one pure function with unit tests (`parse_launch_args`), next to `is_dry_run`.
   - Special helper flags (see `09-restore-point.md`) are handled **before** the Tauri builder and the single-instance plugin, so an elevated helper run never forwards to, or focuses, the running window.
   - Regression test for #3: the persisted shutdown timer is armed exactly once (re-arm happens in the one instance; the forwarded launch does not touch it). Update #3 with the result.
2. **Keep-awake** — `docs/modules/06-keep-awake.md`.
3. **File unlocker** — `docs/modules/07-file-unlocker.md`.
4. **Empty-folder hunter** — `docs/modules/08-empty-folders.md`.
5. **Restore point** — `docs/modules/09-restore-point.md`.
6. Docs: CHANGELOG-DEV entry per task, ROADMAP rows moved to "shipped in v0.2.0", README feature grid (EN + `README.tr.md`), CHANGELOG.md `Unreleased` section.

## Out of scope (do not build)
Module enable flags (#8), sticky notes (#9), legacy panels (#13), anything in v0.3.0. Tray menu entries for the new modules. Forced handle closing or force-kill of any kind.

## Cross-cutting rules
- Every new module: registry line + route + i18n keys in **both** EN and TR + sidebar icon. The wiring test that invokes every registered command must cover the new commands.
- New Windows API surface goes in `core/platform/*` behind a trait so the module logic is unit-testable with a fake (same pattern as `process.rs` / `registry.rs`).
- No network. No new persisted state unless the module spec says so.
- Every user-facing list item carries a human-readable reason.

## Acceptance
`cargo build --release`, `cargo clippy -- -D warnings`, `cargo test`, `npm run build` all green on the branch; CI green on the PR. Each module reachable from the sidebar and from `--module <id>`. Live check of each new page in the running app (screenshots attached to the PR). Second launch focuses the first window.
