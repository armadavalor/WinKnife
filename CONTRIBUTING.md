# Contributing to WinKnife

Thanks for helping. WinKnife's core promise is **analyze first, never delete
without consent, everything reversible**. Every contribution is judged against
that promise before anything else.

## Ground rules

- **No permanent deletion of user files.** Destructive actions go through the
  quarantine service (`docs/ARCHITECTURE.md` §6). The only exception is
  emptying the Windows recycle bin, which the user must tick and confirm
  separately.
- **Protected paths are absolute.** `C:\Windows`, `C:\Program Files*`, user
  profile roots and `brain::protected_paths()` never get a Safe rating or a
  delete action. Enforce it in code, not only in rule data.
- **Scans are read-only.** `scan()` and `analyze()` must not modify the
  filesystem or registry.
- **No network calls** without explicit user opt-in.
- **Every recommendation carries a human-readable reason.**
- System changes (registry, services, settings) must record the original value
  and offer undo. Link to the documentation that explains the change.

The full list of what we will not build (registry "optimizer", driver updater,
RAM booster, …) is in [docs/modules/BACKLOG.md](docs/modules/BACKLOG.md).

## Before you start

1. Open or pick an issue. For a new tool, use the **Tool proposal** template so
   scope and safety can be agreed before code is written.
2. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
   [docs/CONVENTIONS.md](docs/CONVENTIONS.md).
3. Issues labelled `good first issue` are small and well scoped.

## Development setup

Requirements: Windows 10/11, Rust stable (MSVC), Visual Studio C++ Build Tools,
Node.js 24, WebView2 runtime.

~~~powershell
npm ci
npm run tauri dev
~~~

## Checks (all must pass before a PR is merged)

~~~powershell
npm run check
npm run test:ui
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
~~~

Link safety tests need Windows Developer Mode or an elevated session.

New logic needs unit tests. Rule scoring, path classification and quarantine
logic are mandatory test targets. UI changes need a screenshot in the PR.

## Commits and pull requests

- Commit format: `<type>: <description>` where type is one of `feat`, `fix`,
  `refactor`, `docs`, `test`, `chore`, `perf`, `ci`. An optional scope is fine:
  `fix(startup): …`.
- One logical change per commit; keep PRs focused.
- Add a short entry to `docs/CHANGELOG-DEV.md` for non-trivial changes and to
  `CHANGELOG.md` under *Unreleased* for user-visible ones.
- Fill in the PR template, including the safety checklist.

## Reporting security issues

Do not open a public issue. See [SECURITY.md](SECURITY.md).

## License

By contributing you agree that your contribution is licensed under the
[MIT License](LICENSE).
