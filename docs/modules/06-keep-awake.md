# Module: keep-awake (Phase 5)

Purpose: "do not sleep for the next N minutes" with a visible countdown. Small, demo-able, zero risk: nothing is written to disk or registry.

## Behavior
- Durations: presets 30m / 1h / 2h / 4h, plus custom minutes. Maximum 24h; there is no "forever" mode (reject with a friendly message, same wording style as the shutdown timer).
- Option "keep the screen on too" (off by default) → adds `ES_DISPLAY_REQUIRED`.
- Stop button always visible. Starting again while active replaces the deadline (no confirm needed; it is harmless).
- Countdown follows the wall-clock deadline, not tick counting (same rule as shutdown timer).
- Nothing persisted: closing the app ends keep-awake. The page says so in one line.
- If a shutdown/sleep timer is also armed, show a one-line hint: the scheduled action still happens; keep-awake only stops idle sleep.

## Execution detail
- `SetThreadExecutionState` is **per thread**. Own a dedicated worker thread for the lifetime of a session: it sets `ES_CONTINUOUS | ES_SYSTEM_REQUIRED [| ES_DISPLAY_REQUIRED]`, waits on a channel/condvar until the deadline or a stop message, then calls `SetThreadExecutionState(ES_CONTINUOUS)` to clear and exits. Never set it from a Tauri command thread or the async runtime.
- Wrap the call in `core/platform/power.rs` (or a sibling) behind a trait; the module logic is tested with a fake that records calls.
- App exit path stops the session cleanly (Drop of the state clears the flag).

## Trait mapping
`has_scan: false`. Commands: `keep_awake_start(minutes, display)`, `keep_awake_stop()`, `keep_awake_status() -> {active, deadline, display}`. Event `keep-awake:changed` on start/stop/expiry (no per-second event; the UI computes the countdown from `deadline`).

## Acceptance
- Unit: duration validation (0, >24h, custom), start→stop clears the flag, expiry clears the flag, restart replaces the deadline, all through the fake.
- Manual once: `powercfg /requests` lists WinKnife under SYSTEM while active and not after stop. Note result in CHANGELOG-DEV.
