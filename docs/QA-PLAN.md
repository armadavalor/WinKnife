# QA plan

What gets tested where, on which kind of dirty machine, and what is still
only checked by hand. Three layers:

| Layer | Runs | Where |
|---|---|---|
| `cargo test` | every push/PR (`ci.yml` → build job) | GitHub windows-latest |
| CDP e2e (`scripts/qa/e2e.mjs`) | PRs to main + manual `workflow_dispatch` (`ci.yml` → e2e job) | GitHub windows-latest, fixture profile `C:\Users\QA` |
| Manual in Sandbox | before a release, or when a cell below says so | Windows Sandbox via `scripts/qa/Start-QASandbox.ps1` |

**Never run destructive flows on a real PC.** The e2e suite really cleans and
restores. It is only safe on a throwaway runner or inside Windows Sandbox.
The fixture generator refuses to write into the real profile unless
`-DisposableMachine` is passed (the Sandbox bootstrap passes it).

## The dirty-machine fixture

`scripts/qa/New-DirtyProfile.ps1 -Root <dir> [-TempFiles N]` builds a fake
profile under `<dir>` and writes `<dir>\winknife-fixture.json` (paths of every
sentinel, counts, lock-helper PID). It contains:

- `AppData\Local\Temp` with N old files (default 5000; 100000 = "huge Temp"),
  plus one fresh file that must survive (rule minimum age).
- A recent download that must survive.
- Duplicate sets (≥ 1 MB, the duplicates minimum) in Downloads/Documents/Pictures.
- An empty-folder tree, plus a sibling folder that holds one file (must be kept).
- A unicode file name (built from code points; PS 5.1 safe) and a path longer
  than `MAX_PATH`.
- A file held open exclusively by a helper process (unlocker + locked-file cases).
- A directory with a deny ACE for the current user (access denied).
- A junction loop, a junction into `C:\Windows`, a symlink (when the OS allows).

## Windows Sandbox

1. Enable the **Windows Sandbox** optional feature yourself (Settings > Apps >
   Optional features > More Windows features > "Windows Sandbox"), then reboot.
   Windows 10/11 Pro, Enterprise or Education only. Nothing here does it for you.
2. Get an installer: a release `WinKnife_*_x64-setup.exe`, the CI
   `winknife-installer` artifact, or a local `npm run tauri build`.
3. Run:

   ```powershell
   .\scripts\qa\Start-QASandbox.ps1 -InstallerDir <folder with *-setup.exe> -TempFiles 100000
   ```

   It fills the host paths in `scripts/qa/winknife.wsb` (scripts and installer
   are mapped **read-only**), opens the sandbox, and the logon command
   (`sandbox-bootstrap.ps1`) installs WinKnife silently, dirties the sandbox
   profile and launches the app. Log: `Desktop\qa-bootstrap.log` inside the
   sandbox. Networking is off. Closing the sandbox throws everything away.
4. Work through the "manual in Sandbox" cells below.

## Running the e2e suite

CI: Actions → CI → Run workflow (optional `temp_files`, e.g. 100000). Results
and failure screenshots are uploaded as the `e2e-results` artifact.

Locally, only in a VM/Sandbox (see the warning above): install WinKnife, run
the generator, redirect `USERPROFILE`/`LOCALAPPDATA`/`APPDATA`/`TEMP`/`TMP` to
the fixture root, open the WebView2 DevTools port for `winknife.exe` (the
`AdditionalBrowserArguments` policy, as in the e2e job), start the app, then:

```powershell
node scripts/qa/e2e.mjs C:\Users\QA\winknife-fixture.json e2e-out
```

`WINKNIFE_QA_ONLY=<name prefix>` runs a subset. A test marked `knownIssue`
reports `xfail` (expected) or `xpass` (fixed: drop the marker); neither fails
the run.

## Matrix

Legend: **E2E** = a test name from `scripts/qa/e2e.mjs`; `snake_case` = a
`cargo test` name; **Sandbox** = manual in Windows Sandbox on the fixture;
— = does not apply.

### Cleaner

| Scenario | Coverage |
|---|---|
| Empty | `an_empty_recycle_bin_produces_no_finding`, `a_missing_root_yields_an_empty_walk` |
| Huge (100k+) | E2E "cleaner: scan, clean safe items…" with `temp_files=100000`; Sandbox (100k default). Known hang on huge Temp, fixed separately |
| Locked files | `a_locked_file_is_skipped_and_reported`, `a_locked_file_is_gated_to_red_even_with_a_safe_rule`, `a_lock_probe_reporting_in_use_blocks_every_move` |
| Access denied | `an_unreadable_directory_is_recorded_in_the_summary`; Sandbox (deny-ACE dir) |
| Long/unicode paths | `a_rename_handles_paths_longer_than_max_path`; Sandbox (unicode + long sentinels) |
| Junctions/symlinks | `junctions_are_never_followed`, `a_link_pointing_into_a_protected_directory_is_refused`, `a_walk_root_that_is_a_junction_into_a_protected_location_is_refused`, `junctions_are_never_quarantined` |
| Cancel mid-run | `cancellation_stops_a_scan_and_says_so`, `cancellation_stops_the_walk`, `a_running_scan_can_be_cancelled_and_a_finished_one_cannot`. **No UI Stop** (audit A3) |
| Undo/restore | E2E "quarantine: restore the cleaner batch" |
| Restart mid-op | Sandbox: kill the app during Clean up, reopen, check quarantine ledger vs. disk |

### Quarantine (restore / purge)

| Scenario | Coverage |
|---|---|
| Empty | Sandbox (fresh profile: drawer shows the empty state) |
| Huge | Sandbox (clean the 100k Temp, restore the whole batch) |
| Locked files | `a_restore_never_replaces_an_occupied_destination`; Sandbox (lock the destination, restore) |
| Access denied | `a_failed_restore_puts_the_ledger_row_back`, `purge_refuses_paths_outside_the_quarantine_root` |
| Long/unicode paths | `a_rename_handles_paths_longer_than_max_path` |
| Junctions/symlinks | `junctions_are_never_quarantined`, `a_directory_containing_a_junction_is_never_moved_across_volumes`, `a_source_directory_swapped_for_a_junction_is_never_walked_through` |
| Cancel mid-run | — (restore/purge have no cancel; audit A5) |
| Undo/restore | E2E "quarantine: restore the cleaner batch"; `a_whole_batch_restores_at_once`, `a_partial_restore_reports_what_stayed_behind`, `a_restored_item_cannot_be_restored_twice`, `nothing_is_purged_before_its_time`, `a_purged_item_leaves_no_ledger_row`, `a_purge_names_the_batches_it_touched` |
| Restart mid-op | `reopening_an_existing_database_does_not_re_run_migrations`, `a_version_3_database_gains_the_restore_refused_marker`; Sandbox (kill during restore, reopen) |

### Duplicates

| Scenario | Coverage |
|---|---|
| Empty | Sandbox (profile with no ≥ 1 MB files) |
| Huge | Sandbox (100k profile; Pictures/Downloads sets) |
| Locked files | Sandbox (locked sentinel is in a duplicate set; check it is reported, not silently dropped — audit A8) |
| Access denied | Sandbox (deny-ACE dir) |
| Long/unicode paths | Sandbox |
| Junctions/symlinks | `a_hardlinked_pair_is_excluded_and_reported`, `a_link_inside_a_real_set_collapses_without_losing_the_set` |
| Cancel mid-run | `cancellation_before_hashing_stops_the_walk`, `cancellation_during_hashing_stops_the_scan`; E2E "duplicates: Stop…" (**xfail**, audit A2) |
| Undo/restore | Sandbox (move copies to quarantine, restore from drawer) |
| Restart mid-op | Sandbox |
| Happy path | E2E "duplicates: finds the fixture's copy sets" |

### Empty folders

| Scenario | Coverage |
|---|---|
| Empty | `a_missing_root_yields_an_empty_walk`, `an_empty_scan_root_is_reported_but_a_drive_root_has_no_parent` |
| Huge | E2E "empty folders: Stop on a whole-drive scan" (walks `C:\`) |
| Locked files | — (folders only) |
| Access denied | `an_access_denied_subtree_is_treated_as_not_empty` |
| Long/unicode paths | Sandbox |
| Junctions/symlinks | `a_junction_is_not_followed_and_not_reported` |
| Cancel mid-run | `cancelling_stops_the_scan_with_no_findings`; E2E "empty folders: Stop…" |
| Undo/restore | Sandbox (remove → restore from drawer) |
| Restart mid-op | Sandbox |
| Happy path | E2E "empty folders: finds the fixture tree…"; `a_nested_empty_tree_reports_only_its_top` |

### Treemap

| Scenario | Coverage |
|---|---|
| Empty | `an_empty_directory_is_still_a_node` |
| Huge | E2E "treemap: map C:, Stop…"; Sandbox (map the 100k profile to the end) |
| Locked files | — (sizes only) |
| Access denied | Sandbox |
| Long/unicode paths | Sandbox |
| Junctions/symlinks | `a_junction_is_drawn_but_its_target_is_counted_once` |
| Cancel mid-run | `a_cancelled_scan_says_so`; E2E "treemap: map C:, Stop…" |
| Undo/restore | — (read-only) |
| Restart mid-op | Sandbox (kill while mapping; reopen; cache must not be half-written — audit A10) |

### Startup

| Scenario | Coverage |
|---|---|
| Empty | E2E "startup: page loads" (fixture profile has no user entries) |
| Huge | — |
| Locked files | Sandbox |
| Access denied | `a_startup_folder_shortcut_windows_owns_is_refused_by_the_toggle_too` |
| Long/unicode paths | Sandbox |
| Junctions/symlinks | — |
| Cancel mid-run | — (no long operation) |
| Undo/restore | `a_startup_folder_shortcut_is_moved_and_moved_back_intact` |
| Restart mid-op | Sandbox (disable, reboot sandbox app, check state) |

### Shutdown timer

| Scenario | Coverage |
|---|---|
| Empty | `a_timer_longer_than_a_day_is_refused_with_a_friendly_message` |
| Cancel mid-run | Sandbox (start, cancel, confirm no shutdown) |
| Undo/restore | Cancel = undo; Sandbox |
| Restart mid-op | `a_persisted_timer_with_a_future_target_is_rearmed`, `a_persisted_timer_whose_target_has_passed_is_discarded`, `a_tick_long_after_the_target_fires_once` |
| Other scenarios | — |

### Restore point

| Scenario | Coverage |
|---|---|
| Happy path | E2E "restore-point: page loads"; Sandbox (create one; needs UAC) |
| Access denied | Sandbox (decline UAC; UI must leave busy state — audit A11) |
| Cancel mid-run | — (no cancel; audit A11) |
| Other scenarios | — |

### Keep awake

| Scenario | Coverage |
|---|---|
| Happy path / cancel | E2E "keep awake: start and stop" |
| Restart mid-op | `restarting_replaces_the_deadline_and_uses_a_fresh_thread`; Sandbox (kill app while awake; sleep must be allowed again) |
| Other scenarios | — |

### File unlocker

| Scenario | Coverage |
|---|---|
| Locked files | E2E "file unlocker: names the process holding the locked file"; `a_file_held_open_exclusively_is_reported_in_use` |
| Access denied | Sandbox (file held by an elevated process) |
| Long/unicode paths | Sandbox |
| Cancel mid-run | — (no cancel; audit A12/A13) |
| Undo/restore | — (closing a program is not reversible; Sandbox checks the confirmation) |
| Other scenarios | — |

### Legacy panels

| Scenario | Coverage |
|---|---|
| Happy path | E2E "legacy-panels: page loads" |
| Other scenarios | — (shortcuts to Windows tools) |

## Audit: long operations that can leave the UI spinning

Audit only (2026-10-17); nothing below is fixed in this change. A7–A13 are grouped in #41. Filed as GitHub
issues where noted. A3/A4 belong to the cleaner scan-hang fix and are handed
to that session.

| # | Sev. | Finding | Where |
|---|---|---|---|
| A1 (#37) | high | Sync commands run under the global DB mutex with no timeout; one slow command (execute, treemap scan, startup scan, restore/purge) blocks every other command, so unrelated pages spin. | `core/db.rs:179` (`Database::with`), `commands.rs` |
| A2 (#38) | high | Duplicates Stop cancels `this.result?.scanId`: null on the first scan, the *previous* scan on "Scan again". Stop never cancels; UI sits on "Stopping…" until the scan ends by itself. Verified. Empty folders does it right (`runningScanId` from progress). | `src/lib/stores/duplicates.svelte.ts` `stop()` |
| A3 | high | Cleaner has no Stop button although `cancel_scan` exists; a slow scan can only be ended by killing the app. | `src/routes/cleaner/+page.svelte` |
| A4 | high | Cleaner `attachProgress` is awaited outside the `try`, so a failure there leaves `busy` true forever; the blocking walk is not on `spawn_blocking`. | `src/lib/stores/scan.svelte.ts`, cleaner command |
| A5 (#39) | high | `execute`, restore and purge have no cancel, progress or timeout; restore/purge have no busy flag (double-click runs twice). | `core/executor.rs`, `core/quarantine.rs` |
| A6 (#40) | high (inferred) | No `catch_unwind` around command bodies: a panic likely leaves the JS promise pending and the spinner on. | `commands.rs` |
| A7 (#41) | med | `full_hash` reads the whole file without checking the cancel flag; Stop waits for the largest file. | `core/content_hash.rs` |
| A8 | med | Unreadable files are dropped from duplicate sets without a warning. | `modules/duplicates.rs` |
| A9 | med | Empty-folders Stop only works after the first progress event arrives. | `src/lib/stores/empty_folders*` |
| A10 | med | Treemap cache save may not be transactional; a kill mid-save could leave a partial cache. | `modules/treemap.rs` |
| A11 | med | Restore point waits up to 5 min on UAC/WMI with no cancel; `busy` is not reset on panic. | `modules/restore_point.rs` |
| A12 | med | `RmShutdown` (close the program holding a file) has no timeout. | `core/platform/restart_manager.rs` |
| A13 | low | Unlocker query has no timeout. | `core/platform/restart_manager.rs` |
