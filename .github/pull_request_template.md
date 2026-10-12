## What and why

<!-- One or two sentences. Link the issue: Closes #… -->

## How it was tested

<!-- Commands run and their result lines. Screenshot for UI changes. -->

## Safety checklist

- [ ] Scans/analysis stay read-only.
- [ ] No file is deleted permanently outside quarantine; system changes record the original value and can be undone.
- [ ] Protected paths cannot receive a Safe rating or an action.
- [ ] No new network access (or it is opt-in and documented).
- [ ] Every new recommendation has a human-readable reason.
- [ ] `npm run check`, `npm run test:ui`, `cargo clippy -- -D warnings` and `cargo test` pass.
- [ ] `CHANGELOG.md` (user-visible) and/or `docs/CHANGELOG-DEV.md` updated.
