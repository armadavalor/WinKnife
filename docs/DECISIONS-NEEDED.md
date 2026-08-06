# Decisions needed

Open scope questions and contradictions found while implementing. Format:

`- [ ] YYYY-MM-DD (phase N): question — safest interpretation taken`

---

- [x] 2026-08-06 (phase 0): `ARCHITECTURE.md` §1 says `main.rs` holds the Tauri
      builder and command registration, but the Tauri v2 template puts `run()`
      in `lib.rs` (required for the `mobile_entry_point` attribute and for unit
      tests to see the crate) — kept the template layout: `main.rs` is a
      one-line shim calling `cleanify_lib::run()`, and the builder plus
      `generate_handler!` live in `lib.rs`.
      **RESOLVED (architect, 2026-08-06): template layout accepted;
      ARCHITECTURE §1 updated to match.**
- [x] 2026-08-06 (phase 0): the `ToolModule::analyze` signature in
      `ARCHITECTURE.md` §2 takes `&Brain`, but the brain is out of scope until
      phase 1 — kept the signature verbatim and added an empty placeholder
      `Brain` marker type in `core/brain/mod.rs` with no rules, no scoring and
      no protected paths, so phase 1 fills in a body without changing any
      module signature.
      **RESOLVED (architect, 2026-08-06): correct call — signature stability
      over premature logic. Phase 1/2 fill the body.**
- [x] 2026-08-06 (phase 0): `FindingKind` variants are not enumerated anywhere
      in the specs — defined the minimum set implied by the module specs
      (`File`, `Directory`, `RecycleBinItem`, `RegistryEntry`, `ScheduledTask`,
      `Other`); expect phases 1–3 to extend it.
      **RESOLVED (architect, 2026-08-06): set approved; extend per phase as
      needed, never remove variants.**
- [ ] 2026-08-06 (phase 0): the MIT `LICENSE` needs a copyright holder but no
      name/entity has been chosen (see `PLAN.md` §7, the product name is still
      a codename) — used "Cleanify contributors"; update when the project name
      and owner are final.
- [ ] 2026-08-06 (phase 0): the CI badge in `README.md` needs a GitHub
      owner/repo slug, which does not exist yet — used a `OWNER/cleanify`
      placeholder; the workflow itself is unverified until a remote exists.
- [x] 2026-08-06 (phase 0): `CONVENTIONS.md` requires user-facing strings to go
      through `t()`, but module display names and descriptions come from Rust
      `ModuleManifest`s — the UI looks up `module.<id>.name` /
      `module.<id>.description` in the dictionary and falls back to the English
      string supplied by Rust, so a Turkish dictionary stays a drop-in.
      **RESOLVED (architect, 2026-08-06): convention approved — dictionary
      override with manifest fallback is the standing i18n rule for manifests.**

- [ ] 2026-08-06 (phase 1): the `quarantine` table has `restored_at` but no
      `purged_at`, so a purge has to choose between deleting the row (losing
      the audit trail in the database) and adding a migration — took the
      boring option: **purge deletes the row**, and the rolling `tracing` log
      keeps the audit trail with the original path, size and reason. Revisit
      if the drawer ever needs to show purge history.
- [ ] 2026-08-06 (phase 1): module spec 02 lists the recycle bin as a rule
      pack, but the pack format is path-and-glob based and the bin is a shell
      namespace object — modelled it as a pack with the sentinel path
      `shell:RecycleBinFolder` and a dedicated `empty_recycle_bin` action that
      the parser only accepts inside the `recycle-bin` pack. The brain skips
      the protected-path and lock gates for that finding kind, since neither
      question is meaningful for a shell object.
- [ ] 2026-08-06 (phase 1): the cleaner walks directories but never proposes
      one, so an emptied temp folder keeps its (now empty) subdirectories —
      the safest reading of "never delete user files", since a directory can
      gain a file between analyze and execute. Quarantine itself does handle
      directories, so lifting this is a cleaner-side change only.
- [ ] 2026-08-06 (phase 1): `docs/RULES-FORMAT.md` documents `score.base` and
      the 85/40 bucket thresholds, but the phase-1 brief says "no scoring yet:
      rule → fixed bucket" — parse and validate `score.base` (so packs written
      today stay valid) and ignore it, using `score.max_bucket` as the bucket.
      Phase 2 turns it into the real pipeline.
- [ ] 2026-08-06 (phase 1): module spec 01 asks for warning notifications
      "with a cancel action", but Tauri v2's notification plugin has no
      reliable action buttons on Windows 10 — the notification is
      informational and names the tray and the route as the ways to cancel.
      Revisit if the plugin gains Windows action support.
- [ ] 2026-08-06 (phase 1): a rule whose `%VAR%` is unset on this machine
      cannot match anything — reported it in `ScanResult.skipped_rules` and
      shown in the UI rather than dropped silently, so a partial scan says it
      was partial. A *malformed* pack still refuses to start the app.

Open items awaiting the producer (not blocking implementation):
- LICENSE copyright holder (currently "Cleanify contributors") — decide with
  the final product name.
- GitHub owner/repo slug for the CI badge — decide when the remote is created;
  CI stays unverified until first push.
