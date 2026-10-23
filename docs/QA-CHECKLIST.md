<!-- durum: aktif -->
# QA/QC checklist (Polish item 9, 2026-10-20)

This checklist covers the delete/clean/quarantine paths and the search/scan paths.
Each case gives what should happen, how it is checked, and a status:

- **OK**: covered by a passing test, named in the case.
- **BUG → fixed**: found in this pass and fixed. The failing test came first.
- **BUG → decision**: found, but the fix touches a destructive path. Escalated to `docs/DECISIONS-NEEDED.md`.
- **UNTESTED**: no automated check exists yet, and no manual run was made in this pass.

Where the checks run:

- **Rust**: `cargo test` in `src-tauri`.
- **UI**: `npm run test:ui`.
- **e2e**: `scripts/qa/e2e.mjs`, which runs on CI for pull requests to main.

## A. Clean, quarantine, restore, purge

| # | Case | Expected | How it is checked | Status |
|---|---|---|---|---|
| A1 | Clean moves items into quarantine; nothing is deleted. | Every item is a ledgered move; protected and fresh items are untouched. | e2e `cleaner: scan, clean safe items, nothing protected or fresh touched` | OK |
| A2 | Restore a whole batch. | Byte-identical content returns to the original path. | Rust `round_trip_restores_byte_identical_content`, `a_whole_batch_restores_at_once`; e2e `quarantine: restore the cleaner batch` | OK |
| A3 | Restore onto an occupied original path. | Nothing is overwritten. The item comes back as `name (restored).ext`, and the report says so. | Rust `restoring_onto_an_occupied_path_never_overwrites`, `a_restore_never_replaces_an_occupied_destination`, `collision_naming_walks_up_until_it_finds_a_free_name`; e2e `quarantine: restore onto an occupied destination loses nothing` | OK |
| A4 | Restore when the original parent folder was deleted. | The parent is recreated and the item restored. | Rust `restoring_recreates_a_deleted_parent_directory` | OK |
| A5 | Restore when a *file* now stands where a parent folder was. | The restore fails for that item, the ledger row goes back, and the item stays held. | Rust `a_restore_onto_a_file_where_the_parent_was_keeps_the_item_held` | OK |
| A6 | Restore the same item twice. | The second restore is refused. | Rust `a_restored_item_cannot_be_restored_twice` | OK |
| A7 | Restore while another restore runs. | Refused with Busy; one restore applied. | e2e `quarantine: a second restore while one runs is refused (Busy)…`, `…drawer shows the Busy refusal` | OK |
| A8 | Restore a held file that something has locked. | That item fails with a reason; the rest of the batch restores. | Rust `a_locked_item_fails_with_a_reason_while_its_batch_restores` | OK |
| A9 | Process killed between the restore claim and the move. | The item should come back as held, or be freed by purge. | Fixed (K-017): at startup `Quarantine::recover_interrupted_restores` clears `restored_at` on claimed rows whose quarantine copy still exists inside the root, before retention runs. Tests: `a_restore_killed_after_its_claim_is_held_again_at_startup`, `a_finished_restore_is_left_alone_at_startup`, `a_claimed_row_pointing_outside_the_quarantine_root_is_not_trusted`. | OK |
| A10 | Purge by retention and by the size cap. | Only expired items, oldest first; no ledger row left behind; never outside the quarantine root. | Rust `retention_purge_removes_only_expired_items`, `the_size_cap_purges_oldest_first`, `a_purged_item_leaves_no_ledger_row`, `purge_refuses_paths_outside_the_quarantine_root` | OK |
| A11 | Clean a file that is locked. | The file is skipped and reported; the batch continues. | Rust `a_locked_file_is_skipped_and_reported`, `a_lock_probe_reporting_in_use_blocks_every_move`; e2e `file unlocker: names the process holding the locked file` | OK |
| A12 | Long paths (over 260 characters, `\\?\`). | Clean and restore keep the exact name. | Rust `a_rename_handles_paths_longer_than_max_path`, `a_verbatim_root_is_normalised_rather_than_refused`; e2e `cleaner: unicode and >260-char paths go to quarantine and come back by exact name` | OK |
| A13 | Long paths in a cross-volume copy, `destroy`, or the lock probe. | Same as A12. | Rust `a_cross_volume_copy_preserves_a_path_longer_than_max_path`, `a_purge_removes_a_quarantined_path_longer_than_max_path`, `the_clean_lock_probe_reads_a_path_longer_than_max_path` | OK |
| A14 | Cancel a clean mid-run (#59). | What already moved stays in quarantine; the rest is skipped; the run is recorded as cancelled. | Rust `a_batch_stopped_before_it_starts_moves_nothing`, `a_stop_mid_batch_keeps_what_moved_and_skips_the_rest`, `a_cancelled_run_is_recorded_as_cancelled`; e2e `cleaner: Stop during a clean keeps what moved and leaves the rest (QA A14)` | OK (backend); UI e2e runs in the CI e2e job |
| A15 | Cancel a restore mid-run (#59). | Restored items stay restored; the rest stay held. | Rust `a_stopped_batch_restore_leaves_the_rest_held`; e2e `quarantine: Stop during a batch restore keeps the rest held and returns to idle (QA A15)` | OK (backend); UI e2e runs in the CI e2e job |
| A16 | Partial failure is reported honestly. | The report counts what moved and what stayed, with reasons. | Rust `one_failure_does_not_stop_the_batch`, `a_partial_take_reports_what_stayed_behind`, `a_partial_restore_reports_what_stayed_behind`, `a_partial_delete_reports_what_it_removed_and_what_it_left` | OK |
| A17 | Partial failure during a purge. | The purge reports the items it could not remove and keeps their rows. | Rust `a_partial_purge_reports_the_locked_item_and_keeps_its_row` | OK |
| A18 | Real move across two volumes. | Copy, verify, delete the source; nothing lost on failure. | Rust `a_cross_volume_move_reports_an_occupied_destination_as_occupied` (simulated). No real second volume on CI. | UNTESTED |
| A19 | Execute UI after a partial failure. | The UI shows the failed rows and keeps the rest of the state intact. | e2e `cleaner: a file locked before the scan is listed under 'Left alone' with its reason and has no tick (QA A19)` and `cleaner: a file locked after the scan is reported with its reason, stays in place, and the rest moved (QA A19)`; Rust `a_locked_file_is_skipped_and_reported` | UI e2e runs in the CI e2e job |
| A20 | Emptying the recycle bin (the one permanent delete). | The row starts unticked; the backend refuses without the separate confirmation flag. | Existing Phase 4 tests; unchanged in this pass. | OK |

## B. Search and scan

| # | Case | Expected | How it is checked | Status |
|---|---|---|---|---|
| B1 | Cleaner: Stop. | The scan stops and says so. Stop stays disabled until the first event names the scan. | Rust `cancellation_stops_a_scan_and_says_so`, `cancelling_analysis_keeps_only_the_judged_prefix` | OK |
| B2 | Empty folders: Stop on a big folder. | Back to idle with a stopped note. An initial event names the scan at once. | Rust `cancelling_stops_the_scan_with_no_findings`; e2e `empty folders: Stop on a big folder` | OK |
| B3 | Duplicates: Stop pressed before the first progress event. | The Stop is sent as soon as an event names the scan. | UI `scan-stop-regression.mjs`. Backend: a tree with no qualifying file sent **no** event, so the Stop waited forever. This was the e2e flake (see B8). | BUG → fixed: Rust `a_scan_reports_its_root_before_finding_any_candidate` |
| B4 | Duplicates: a Stop accepted after the pipeline's last cancel check. | `cancel_scan` said true, so the result must say stopped and offer no sets. | Before the fix, the full result came back unstopped. | BUG → fixed: Rust `finish_reports_a_cancel_that_the_scan_never_saw`, `a_stopped_scan_keeps_no_half_answer` |
| B5 | Duplicates: the scan store records a cancelled run. | `ScanResult.cancelled` matches the scan. | Before the fix, it was always stored as `false`. | BUG → fixed (`commands.rs`, `cancelled: scan.cancelled`) |
| B6 | Duplicates: a late event after the scan finished. | Ignored; the listener is detached in `finally`. | UI `scan-stop-regression.mjs` | OK |
| B7 | Duplicates: rescan after Stop and after completion. | State is reset: selection, keeper overrides, id, and the stopping flag. | Code reading of `scan()`; UI `scan-stop-regression.mjs` (stop, then a fresh run) | OK |
| B8 | e2e flake `duplicates: Stop cancels the scan` (the old name was `…returns to idle`). | Passes every time, without raising timeouts. | **Root cause:** the e2e clicks Stop before any progress event, so the UI defers the cancel until an event names the scan. The backend's first duplicates event came only with the first qualifying candidate (at least the minimum size, not hidden, not protected). A slow or sparse start on `C:\hostedtoolcache` meant the cancel went out late, or never, and the run went past the 60 s step timeout. **Fix:** B3. | BUG → fixed |
| B9 | Treemap: Stop pressed before the first progress event. | The Stop is not lost. | Before the fix, `stop()` was a no-op without an id. It now sets `stopping` and sends the cancel on the first event; the button is disabled while stopping. | BUG → fixed: UI `scan-stop-regression.mjs` |
| B10 | Treemap: a late event after the walk finished. | It does not revive the old scan id or the progress count. | Before the fix, the listener set `scanId` unconditionally. | BUG → fixed: UI `scan-stop-regression.mjs` |
| B11 | Treemap: rescan after Stop. | Starts with no old id and no stale stopping flag; `partial` is cleared by a completed rescan. | UI `scan-stop-regression.mjs` | BUG → fixed (the reset was missing) |
| B12 | Treemap: map C:, Stop, the button comes back. | Back to idle. | e2e `treemap: map C:, Stop, button comes back` | OK |
| B13 | Large trees: no UI freeze, progress shown. | Progress events are throttled to 10 per second (the first always passes); hashing runs on rayon off the UI thread; the count and path keep moving. | Rust `a_long_clean_does_not_hold_the_database`; e2e Stop cases on `C:\hostedtoolcache` and C:. No frame-time measurement. | OK (functional); UI frame time UNTESTED |
| B14 | Startup, restore point, file unlocker, keep awake. | No long cancellable scan; not applicable. | — | n/a |
| B15 | Scans are read-only. | No scan or analyze step writes. | The module contract and the duplicates comment "nothing here opens a file for writing"; unchanged. | OK |
