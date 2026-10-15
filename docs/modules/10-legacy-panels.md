<!-- durum: aktif -->
# Module: legacy-panels (v0.2.0 leftover, issue #13)

Purpose: one screen of shortcuts to the classic Windows applets that newer Settings pages hide (`ncpa.cpl`, `sysdm.cpl`, Device Manager, …). Inspired by WinUtil's "legacy panels". Zero risk: WinKnife only opens Windows' own windows; it changes nothing itself.

## Behavior
- A fixed catalogue, grouped: **System**, **Hardware & devices**, **Network**, **Apps & accounts**, **Diagnostics**. Each entry has a name, a one-sentence description of what the window is for, and an Open button.
- The catalogue lives in the backend. The UI asks for it and opens entries **by id only**; it never sends a path or command line. An unknown id is an error.
- Every target is an absolute path under `%SystemRoot%\System32` (no `PATH` lookup, no user-controlled argument). Entries whose file is missing on this edition of Windows are listed with the button disabled and a one-line "not on this edition of Windows".
- Some windows ask for administrator rights themselves (MMC snap-ins, `msconfig`). WinKnife launches through the shell so Windows shows its own UAC prompt; saying No is not an error.
- Read-only for WinKnife: no registry or file writes, nothing persisted, no network.

## Execution detail
- Platform trait `PanelLauncher` in `core/platform/panels.rs`: `exists(path) -> bool` and `open(file, args) -> Result<()>`. Real impl uses `ShellExecuteExW` (verb default) and is gated on `real_os_effects_armed()`, like `reveal_in_explorer`. Tests use a recording fake.
- Targets are built as `control.exe <System32>\x.cpl`, `mmc.exe <System32>\x.msc`, or a System32 exe with no arguments.

## Trait mapping
`has_scan: false`, `has_execute: false`. Commands: `legacy_panels_list() -> [{id, group, name, description, available}]`, `legacy_panels_open(id)`.

## Acceptance
- Unit: ids are unique kebab-case; every entry has a name and description; every target resolves under System32; open of a known id hits the launcher with the expected file and args; unknown id errors and launches nothing; a missing file reports `available: false` and refuses to open; a test build cannot launch anything for real.
- Manual once: each button opens its window on Windows 10/11; UAC No on an MMC entry shows no error.
