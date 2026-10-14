<!-- durum: aktif -->
# Module: startup

Purpose: show everything that runs at boot, estimate impact, disable safely (reversibly).

## Sources to enumerate
1. Registry Run keys: `HKCU/HKLM ...\Software\Microsoft\Windows\CurrentVersion\Run` (+ `RunOnce`, + WOW6432Node)
2. Startup folders: user + common (`shell:startup`, `shell:common startup`)
3. Task Scheduler: logon-trigger tasks (via `schtasks /query /xml` parsing or COM ITaskService wrapper — COM preferred)
4. (Read-only in V1) Services set to Automatic that are third-party — display only, no disable action (too risky for V1; note as V2)

## Disable = reversible, mirroring Windows' own approach
- Registry entries: use the official `StartupApproved` mechanism (`...\Explorer\StartupApproved\Run` binary flag) — same as Task Manager, survives and cooperates with it. Do NOT delete Run values.
- Startup folder: move `.lnk` into module-managed `disabled/` subfolder in app data (recorded in DB, restorable).
- Scheduled tasks: `Enabled=false` via API, never delete.

## Brain integration
- Signature check (Authenticode publisher via wrapper) + known-app list (rule pack `startup-known.json`: publisher/name → category + verdict hint: keep / user-choice / safe-to-disable).
- Impact estimate V1 = honest heuristic tiers only (High/Medium/Low from binary size + disk vs SSD + known-app data). Label it "estimated". Real boot-time measurement is V2 (note in manifest).
- Unknown unsigned entries → NeedsReview with "unsigned, unknown publisher" reason. LLM layer (if enabled) can enrich: "This is the updater for X".

## UI
Table: name, publisher (✓ signed badge), source (registry/folder/task), impact tier, status toggle. Toggle = the execute action (each toggle is an executor action logged to DB, undoable from history). Red-bucket equivalents: OS entries (Windows Defender etc.) shown grayed, toggle disabled, reason shown.

## Acceptance
- Enumerates all 3 sources on a real machine (manual smoke) + fixture-registry unit tests via registry wrapper mock.
- Disable→reboot-sim→re-enable round-trip preserves exact original value/link.
- Task Manager sees our disables (StartupApproved compatibility verified manually once, noted in CHANGELOG-DEV).
