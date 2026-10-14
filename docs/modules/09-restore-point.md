<!-- durum: aktif -->
# Module: restore-point (Phase 5)

Purpose: one button to create a Windows restore point, offered before system changes. WinKnife's own actions are already reversible; this is an extra safety net for users who want one.

## Behavior
- Page shows: whether System Protection looks enabled for the system drive (best-effort read-only check; "unknown" is a valid answer), and a **Create restore point** button with an optional description (default "WinKnife — before changes", max 64 chars, sanitized to printable characters).
- Creating needs administrator rights → UAC prompt. Copy says so before the click.
- Result states, each with a plain sentence: created / skipped because Windows allows only one per 24 hours / System Protection is off (tell the user where to enable it: `SystemPropertiesProtection.exe`, opened only on button click) / UAC declined / failed (with error code).
- Startup page: a small non-blocking link "Create a restore point first" above the toggles → navigates to this module. No modal, no nagging.
- Never change System Protection settings or `SystemRestorePointCreationFrequency`. Read only.

## Execution detail
- Elevation via our **own exe** re-launched with `ShellExecuteExW` verb `runas` and `--create-restore-point "<description>"`; no PowerShell, no scripts. Wait for exit with a timeout (5 min), read the exit code.
- Helper mode is handled at the very top of `run()` (before logging-to-DB, the DB, the Tauri builder and the single-instance plugin): call `SRSetRestorePointW` (`BEGIN_SYSTEM_CHANGE`, `MODIFY_SETTINGS`), then `END_SYSTEM_CHANGE`; map results to documented exit codes (constants shared with the parent). Detect the 24h skip by comparing the newest restore point sequence/creation time before and after (WMI `SystemRestore` in the elevated helper) — if the API cannot tell, report "Windows may have skipped it (24h limit)" honestly instead of claiming success.
- Platform wrapper behind a trait; the parent-side state machine (exit code → UI state) is unit-tested with a fake.
- Commands: `restore_point_status()`, `restore_point_create(description)`. `has_scan: false`.

## Acceptance
- Unit: description sanitizing/length, exit-code → result mapping table, helper-flag parsing (and that it bypasses single-instance).
- Manual once (admin VM or local): create succeeds; second create within 24h reports skipped; UAC "No" reports declined. Record in CHANGELOG-DEV.
