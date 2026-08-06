# Cleanify

[![CI](https://github.com/OWNER/cleanify/actions/workflows/ci.yml/badge.svg)](https://github.com/OWNER/cleanify/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A modular Windows "Swiss army knife" utility: disk cleaning, duplicate finding,
treemap, startup management and a shutdown timer — in one calm window.

> **Status: Phase 1 (first blood).** Two working tools — a shutdown timer and
> a disk cleaner covering the three "safe" rule packs — on top of a quarantine
> service that makes every removal reversible. See `docs/phases/` for the
> roadmap.

## Core promise

**Analyze first, never delete without consent, everything reversible.**

- Nothing is deleted without an explicit confirmation. Everything removable
  goes to a **quarantine** first and can be restored.
- **Offline-first.** The intelligence is a local rule engine. The optional LLM
  layer is off by default and never sees file contents.
- **Every recommendation explains itself** — each one carries a human-readable
  reason.

## Tech stack

| Layer      | Choice                                  |
| ---------- | --------------------------------------- |
| Backend    | Rust, [Tauri v2](https://tauri.app)     |
| UI         | Svelte 5 + TypeScript + Tailwind CSS v4 |
| Storage    | SQLite (rusqlite) + JSON rule packs     |
| CI         | GitHub Actions on `windows-latest`      |

## Development

### Prerequisites

Windows 10 or 11, plus:

| Tool | Version | Notes |
| ---- | ------- | ----- |
| [Rust](https://rustup.rs) | stable, `x86_64-pc-windows-msvc` | `rustup default stable-msvc` |
| [Node.js](https://nodejs.org) | 20 or newer | ships with npm |
| Visual Studio Build Tools | 2019 or newer | "Desktop development with C++" workload |
| [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) | evergreen | preinstalled on Windows 11 and up-to-date Windows 10 |

That is the full list from Tauri's
[Windows prerequisites](https://tauri.app/start/prerequisites/). If
`cargo build` fails with a linker error, the C++ workload is the usual cause.

### First run

```sh
git clone <repo> && cd cleanify
npm install              # UI dependencies
npm run tauri dev        # build everything and open the app
```

The first `cargo` build compiles the whole dependency tree and takes a few
minutes; later builds are incremental.

### Everyday commands

```sh
npm run build            # build the UI only
npm run check            # svelte-check / TypeScript, must be 0 errors

cd src-tauri
cargo test               # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release    # portable exe in target/release/

npm run tauri build      # portable exe + NSIS installer
```

### Running the shutdown timer safely

The shutdown timer really does shut the machine down. Pass `--dry-run` to make
every power action a log line instead:

```sh
npm run tauri dev -- -- --dry-run     # dev
./target/release/cleanify.exe --dry-run
```

The UI shows a banner when the flag is active. **The test suite can never
trigger a real shutdown, sleep or recycle-bin empty**: those calls live behind
traits in `src-tauri/src/core/platform/`, the real implementations refuse to
run under `cfg!(test)`, and tests only ever construct the fakes.

### Where things live

| Path | What |
| ---- | ---- |
| `rules/*.json` | Rule packs — the data the cleaner runs on. See `docs/RULES-FORMAT.md`. |
| `src-tauri/src/core/` | Quarantine, executor, brain, scanner, database, platform wrappers |
| `src-tauri/src/modules/` | One file per tool |
| `src/routes/<module-id>/` | One folder per tool's UI |
| `%APPDATA%\com.cleanify.app\` | Database, quarantine and rolling logs at runtime |

### Adding or editing a rule pack

Edit or add a file in `rules/`, then run `cargo test`. The validate-all-packs
test loads every pack and fails loudly with the pack name and the exact field
if anything is wrong; a separate test proves no rule can point at a protected
path. `rules/rule-pack.schema.json` gives editor completion.

## Tools

| Tool | What it does |
| ---- | ------------ |
| **Disk Cleaner** | Scans temp folders, Explorer's thumbnail/icon caches and the recycle bin. Sorts findings into three buckets — safe, needs review, left alone — each with a plain-language reason. Cleaning moves files to quarantine. |
| **Shutdown Timer** | Schedules a shutdown, restart or sleep by duration or clock time, with a tray countdown, 5- and 1-minute warnings, and a cancel button that is never more than one click away. |

## Adding a tool module

A new tool is **2 files + 1 registry line**:

1. `src-tauri/src/modules/<id>.rs` — implement `ToolModule`.
2. `src/routes/<id>/+page.svelte` — the module's UI route.
3. One line in the `register_modules!` block in
   `src-tauri/src/modules/mod.rs`.

The sidebar, manifests and routing follow from the registry automatically, and
a test fails the build if a registered module has no route.

## Codename

`cleanify` is a **codename**; the public name is not decided yet (candidates
are in `PLAN.md` section 7). The UI reads its display name from a single
constant, `APP_NAME` in `src/lib/branding.ts`. Renaming the project means
touching exactly these places:

| File                            | Field                                          |
| ------------------------------- | ---------------------------------------------- |
| `src/lib/branding.ts`           | `APP_NAME` (all UI strings derive from it)     |
| `src-tauri/tauri.conf.json`     | `productName`, `identifier`, window `title`    |
| `src-tauri/Cargo.toml`          | `package.name`, `lib.name`                     |
| `src-tauri/src/main.rs`         | the `<crate>_lib::run()` call                  |
| `package.json`                  | `name`                                         |
| `README.md`, `LICENSE`          | prose                                          |

## Documentation

- `docs/ARCHITECTURE.md` — system design, module contract, data model
- `docs/CONVENTIONS.md` — code style, testing, commit format
- `docs/RULES-FORMAT.md` — rule pack JSON schema
- `docs/phases/` — phase-by-phase implementation plan
- `docs/CHANGELOG-DEV.md` — development log

## License

MIT — see [LICENSE](LICENSE).
