# WinKnife

<img width="412" height="412" alt="WinKnife Logo gh" src="https://github.com/user-attachments/assets/02b97ebe-a96e-4508-9b20-208a2db247a1" />

[![CI](https://github.com/armadavalor/WinKnife/actions/workflows/ci.yml/badge.svg)](https://github.com/armadavalor/WinKnife/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A Windows utility that brings disk cleaning, duplicate and empty-folder finding,
a disk map, startup management, a file unlocker, keep-awake, restore points and
a shutdown timer into one window.

[ Türkçe ](README.tr.md)

**Status:** nine tools implemented; preparing the first release, v0.2.0.
Release acceptance, including a fresh Windows VM install/restore/uninstall check,
is still pending. There is no promised release date.

| Tool | What works |
| --- | --- |
| Disk Cleaner | Read-only scans, reasons and risk buckets, selected cleanup through quarantine, previous scan summary. |
| Duplicate Finder | Byte-identical duplicate detection and review before quarantining selected copies. |
| Disk map | Folder sizes, drill-down and a treemap to investigate disk usage. |
| Startup | Review login entries, disable and restore supported entries. Impact is estimated, not a measured boot time. |
| Shutdown Timer | Schedule shutdown, restart or sleep; countdown, warnings and cancellation. |
| Empty Folders | Folders with no file at any depth; selected ones move to quarantine. |
| File Unlocker | Shows which program holds a file open; can ask it to close. |
| Keep Awake | Keeps the PC (optionally the screen) awake for up to 24 hours. Nothing is persisted. |
| Restore Point | Shows System Protection status and creates a restore point (UAC prompt). |

## Safety and privacy

- Scans inspect first; file cleanup requires confirmation. Each recommendation explains its reason.
- Ordinary cleaned files go to quarantine and can be restored **while retained**. Retention age and size limits can permanently purge them, including at app startup.
- **Emptying the Windows recycle bin is permanent** and has its own confirmation. It cannot be restored through WinKnife.
- Protected system locations cannot receive a cleanup action. Uncertain items require review or are informational only.
- The current app uses local rules. The optional LLM layer is planned and **not implemented** in this release preparation.
- WinKnife is not a registry cleaner, driver updater, antivirus or a promise of more FPS. See [module backlog](docs/modules/BACKLOG.md).

## Build and run

Windows development requires Rust stable with the MSVC toolchain, Visual Studio
C++ Build Tools, Node.js **24**, and the WebView2 runtime.

From the repository root:

~~~powershell
git clone https://github.com/armadavalor/WinKnife.git
cd WinKnife
npm ci
npm run tauri dev
~~~

Checks and production build:

~~~powershell
npm run check
npm run test:ui
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri build -- --bundles nsis
~~~

Link safety tests require Windows Developer Mode or an elevated test session.
CI must run them without opt-outs. The local opt-out and its limitations are
recorded in [release instructions](docs/RELEASING.md).

The executable is built at `src-tauri/target/release/winknife.exe`; it needs
its `rules/` resources beside it. Use the portable ZIP when distributing it.
Application data remains in Windows roaming app data under `com.winknife.app`;
“portable” means no installer, not that user data travels with the executable.

~~~powershell
npm run tauri dev -- -- --dry-run
~~~

**The flag only disables real power actions.** It does not disable cleaner,
startup edits, recycle-bin emptying or quarantine retention. Use an isolated
VM or the separate QA identity described in [QA notes](docs/POLISH-QA.md).

## Contributing

Small, reproducible fixes, tests, documentation and reviewed rule-pack changes
are welcome. Describe the problem, expected behavior and your verification.
Read [conventions](docs/CONVENTIONS.md) and the relevant
[module specification](docs/modules/) before changing behavior.

For rule packs, read [the format](docs/RULES-FORMAT.md), edit `rules/*.json`,
and run the Rust tests. Protected paths and quarantine guarantees remain
mandatory. Prefer a focused change over unrelated cleanup in the same PR.

| Location | Contents |
| --- | --- |
| `src-tauri/src/core/` | Safety gates, quarantine, database, scanner and platform wrappers |
| `src-tauri/src/modules/` | Tool implementations |
| `src/routes/` | Svelte screens |
| `rules/` | Local rule packs and schemas |
| [Architecture](docs/ARCHITECTURE.md) | Module contract and design |
| [Development log](docs/CHANGELOG-DEV.md) | Completed work and deferrals |
| [Release checklist](docs/RELEASING.md) | Packaging and acceptance |

MIT — see [LICENSE](LICENSE).