# Module: file-unlocker (Phase 5)

Purpose: answer "who is locking this file?" and let the user ask that app to close. Never force anything.

## Behavior
- Input: pick file(s) or one folder (dialog plugin), or drag-drop onto the page.
- Folder: enumerate the files under it with the shared scanner (no junction/reparse traversal), cap at 5 000 files; when capped, say so ("checked the first 5 000 files").
- Result list, one row per process: app name, exe path, PID, kind (window app / console / service / Explorer / critical), which of the chosen files it holds (first 3 + count), and a `reason` sentence.
- Empty result: "Nothing is holding these files open." with a hint that a file may also be read-only or blocked by permissions (link to nothing, just text).

## Actions
- **"Ask to close"** — only for window apps and console apps. Graceful: Restart Manager `RmShutdown` in a fresh session that registers **only that process** (`RM_UNIQUE_PROCESS` = PID + start time, so a reused PID is never hit). Flags: none — **never `RmForceShutdown`**, never `TerminateProcess`, never handle closing.
  Confirm dialog: "<App> will be asked to close. If it has unsaved work, it may ask you to save first."
  Afterwards re-query and report: closed / still running (app refused or asked the user).
- **Services, critical processes (`RmCritical`), Explorer, PID 4 and WinKnife itself**: no action button; InfoOnly with reason (e.g. "Windows service — stop it from Services if you are sure", "Explorer — restarting it is safer than closing it; WinKnife does not do this for you").
- "Open file location" for the exe (existing explorer helper). Nothing else.

## Execution detail
- Platform wrapper `core/platform/restart_manager.rs` behind a trait: `locking_processes(&[PathBuf]) -> Result<Vec<LockInfo>>`, `request_close(UniqueProcess) -> Result<CloseOutcome>`. Always `RmEndSession`, including on error paths (guard type).
- `RmGetList` with `ERROR_MORE_DATA` retry loop (bounded, e.g. 3 tries). Register files in batches (RM has practical limits; batch by 500).
- Per-file attribution: RM reports per session, not per file. When more than one file is chosen, attribute by querying in batches and, for ≤ 50 files, per file; above that, show "holds one or more of these files". Document the choice in CHANGELOG-DEV.
- Module-specific commands, `has_scan: false` in the manifest sense (it is a query, read-only): `unlocker_query(paths)`, `unlocker_request_close(pid, start_time)`. Each close is logged (tracing) with exe path and outcome.

## Acceptance
- Unit (fake RM): classification → action availability table (window/console → closable; service/critical/explorer/self/pid 4 → InfoOnly with reason), PID-reuse guard (start time mismatch → refused), folder cap message, session always ended.
- Integration (real API, CI-safe): a test opens a temp file with an exclusive handle in a child process and `unlocker_query` names that process.
- Manual once: open a .docx in Word, query it, "Ask to close" → Word prompts or closes.
