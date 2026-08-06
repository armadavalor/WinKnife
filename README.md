# Cleanify

[![CI](https://github.com/OWNER/cleanify/actions/workflows/ci.yml/badge.svg)](https://github.com/OWNER/cleanify/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A modular Windows "Swiss army knife" utility: disk cleaning, duplicate finding,
treemap, startup management and a shutdown timer — in one calm window.

> **Status: Phase 0 (skeleton).** The app builds and runs, the module
> architecture is in place, and there are no features yet. See
> `docs/phases/` for the roadmap.

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

Prerequisites: [Rust](https://rustup.rs) (stable, MSVC toolchain),
[Node.js](https://nodejs.org) 20+, and the Microsoft C++ build tools /
WebView2 runtime (Tauri's
[Windows prerequisites](https://tauri.app/start/prerequisites/)).

```sh
npm install
npm run tauri dev        # run the app in development
npm run build            # build the UI only
npm run check            # svelte-check / TypeScript
npm run tauri build      # portable exe + NSIS installer

cd src-tauri
cargo test               # Rust unit tests
cargo clippy -- -D warnings
```

## Adding a tool module

A new tool is **2 files + 1 registry line**:

1. `src-tauri/src/modules/<id>.rs` — implement `ToolModule`.
2. `src/routes/<id>/+page.svelte` — the module's UI route.
3. One line in the `register_modules!` block in
   `src-tauri/src/modules/mod.rs`.

The sidebar, manifests and routing follow from the registry automatically.

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
