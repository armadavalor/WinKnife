# AGENTS.md — Instructions for Codex

Codex works on WinKnife as an **implementing worker and independent reviewer**.
Decisions belong to the maintainer and the maintainer's Claude orchestrators.
Tasks come from them; results go back to them for review.

## Read first

1. `CLAUDE.md` — hard rules and definition of done. They apply to you unchanged.
2. `docs/ARCHITECTURE.md` and `docs/CONVENTIONS.md`
3. The phase brief and module spec named in your task.

## Do not touch unless the task names the file

- Safety-critical core: `src-tauri/src/core/quarantine.rs`, `core/executor.rs`,
  `core/brain/`, `core/path_compare.rs`, `core/scanner.rs`.
- Security configuration: `src-tauri/capabilities/` and the CSP/security
  settings in `src-tauri/tauri.conf.json`.
- Version numbers, `.github/workflows/`, and the architect resolutions in
  `docs/DECISIONS-NEEDED.md`.
- The meaning of user-visible text. Moving or extracting strings is fine;
  rewriting them is not.
- `chatgpt Pocketknife addon.txt` is the maintainer's private file: never edit,
  stage or delete it.

## Mandatory

- The CLAUDE.md hard rules: no permanent deletes outside quarantine purge,
  protected paths never rated Safe, no network calls, read-only scans, and a
  human-readable reason on every recommendation.
- If you changed files, run `npm run check` and `npm run test:ui` before
  finishing, then in `src-tauri`: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets -- -D warnings` and `cargo test`.
  Red means not done. Read-only review tasks skip these unless the task asks.
- Never stop processes by image name (`node.exe`, `cargo.exe`); other work may
  share this machine. Stop only processes you started.
- Link safety tests need Windows Developer Mode or an elevated session. Locally
  you may set `WINKNIFE_ALLOW_SKIP_LINK_TESTS=1`, but report every
  `!! SKIPPED SAFETY TEST` line. Never set it in CI.
- Do not commit, push, tag or publish. Leave the diff in the working tree.
- Stay inside the files named in the task.

## Feedback

Say so when something looks wrong — a missed bug, a risk, a contradiction or a
better option — even outside your task. Put it under "Feedback" in your final
report instead of acting on it. The final decision is the orchestrators'.

## Final report (last message, short)

1. Changed files
2. What was done, in one paragraph
3. Check results (summary lines only)
4. Uncertainties
5. Feedback (optional)
