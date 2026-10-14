<!-- durum: aktif -->
# Phase 0 — Skeleton (compiles from day one)

Goal: a running Tauri v2 app with the module architecture in place and zero features.

## Tasks
1. Scaffold Tauri v2 + Svelte 5 + TS + Tailwind (`npm create tauri-app`), repo hygiene: `.gitignore`, MIT `LICENSE`, stub `README.md`, `docs/CHANGELOG-DEV.md`.
2. Implement `core/module.rs`: `ToolModule` trait, `ModuleManifest`, registry (static vec of boxed modules). Register one `DummyModule` proving the pattern.
3. Shared types: `Finding`, `Recommendation`, `RiskBucket`, `Action`, `ExecReport` (serde) + mirrored `src/lib/types.ts`.
4. `core/db.rs`: rusqlite open-in-app-data + migration runner + migration 001 (tables from ARCHITECTURE §5).
5. `core/events.rs`: typed emit helpers; `commands.rs`: `list_modules` working end-to-end.
6. UI shell: sidebar rendered FROM `list_modules`, dashboard route (placeholder cards), dark theme, `t()` i18n helper wrapping strings.
7. CI: GitHub Actions on `windows-latest` — clippy, test, build. Badge in README.

## Acceptance
App launches; sidebar shows Dummy module from registry; `cargo clippy -D warnings`, tests, and CI green; adding a second dummy module = 2 files + 1 registry line (verify by doing it, then delete).

## Out of scope
Any real scanning, quarantine, brain. No UI polish beyond shell.
