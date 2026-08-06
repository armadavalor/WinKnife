# Module: shutdown-timer (Phase 1 — first end-to-end win)

Purpose: schedule shutdown / restart / sleep with a friendly countdown. Small, demo-able, exercises the module registry + settings + tray without any risk.

## Behavior
- Modes: **at duration** ("in 2h 30m"), **at clock time** ("at 23:30"), **cancel**.
- Actions: shutdown, restart, sleep. (Hybrid-shutdown nuances: use `shutdown /s /t 0` semantics; sleep via `SetSuspendState` through `windows` crate wrapper.)
- Timer lives in Rust (app must keep running; minimize-to-tray). Tray tooltip shows remaining time; tray menu: cancel.
- UI: big countdown, quick presets (30m / 1h / 2h / tonight 23:00), cancel button always visible.
- 5-minute and 1-minute warning notifications (tauri notification plugin) with cancel action.
- Persist active timer in settings → on app restart, re-arm if target still in future, else discard.

## Trait mapping
`scan/analyze` = no-op (manifest capability flag `has_scan: false`). Commands: `shutdown_schedule(mode, action, target)`, `shutdown_cancel()`, `shutdown_status()`. Event: `shutdown:tick {remaining_secs}` (1/sec only while route visible or tray tooltip refresh at 30s).

## Execution detail
At T-0 spawn `shutdown.exe /s /t 0` (or `/r`); do NOT pre-arm Windows' own `shutdown /t <big>` (can't show our countdown, cancel UX is worse). Sleep path calls API directly.

## Edge cases
- System clock changes / resume from sleep: recompute from wall-clock target, not tick counting.
- Duration > 24h: reject with friendly message.
- Two schedules: replacing is fine — confirm dialog.

## Acceptance
- Schedule 1 min → warning fires → machine would shut down (test with `--dry-run` build flag that logs instead of executing — keep this flag for CI).
- Cancel works from route, tray, and notification. Restart-app persistence works.
