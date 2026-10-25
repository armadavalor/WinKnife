<!-- durum: aktif -->
# Phase 7 — Installed apps (C19)

A read-only inventory module (`apps`). No uninstall, no deletion, no network.

## Data

- **Win32:** `Uninstall` keys under HKLM (64-bit), HKLM `WOW6432Node` and HKCU.
  Reads DisplayName, Publisher, DisplayVersion, InstallDate, EstimatedSize,
  InstallLocation. Skips `SystemComponent=1`, entries with no DisplayName and
  update entries (`ParentKeyName`). De-duplicated by name + version.
- **Store/Appx:** HKCU `...\AppModel\Repository\Packages` subkeys (no PowerShell,
  no WinRT). Framework and resource packages are marked. An unresolved
  `@{...}` display name falls back to the package family.
- **Signer:** only Win32 rows with a resolvable exe. Looked up on demand (row
  button, or "check shown" capped at 40). Unsigned reads "needs review".

## Commands

`apps_list`, `apps_signer(id)`, `apps_open_settings`. Paths for signer checks
stay in Rust; the UI sends back only the row id (unknown id -> `unknown_app`).

## Actions

"Open in Settings" (`ms-settings:appsfeatures`) and "Show install folder".

## Tests

Rust unit tests use `MapRegistry` / `MapSignatures` / `FakePanelLauncher`;
`command_wiring.rs` probes the three commands.
