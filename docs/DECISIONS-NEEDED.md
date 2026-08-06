# Decisions needed

Open scope questions and contradictions found while implementing. Format:

`- [ ] YYYY-MM-DD (phase N): question — safest interpretation taken`

---

- [ ] 2026-08-06 (phase 0): `ARCHITECTURE.md` §1 says `main.rs` holds the Tauri
      builder and command registration, but the Tauri v2 template puts `run()`
      in `lib.rs` (required for the `mobile_entry_point` attribute and for unit
      tests to see the crate) — kept the template layout: `main.rs` is a
      one-line shim calling `cleanify_lib::run()`, and the builder plus
      `generate_handler!` live in `lib.rs`.
- [ ] 2026-08-06 (phase 0): the `ToolModule::analyze` signature in
      `ARCHITECTURE.md` §2 takes `&Brain`, but the brain is out of scope until
      phase 1 — kept the signature verbatim and added an empty placeholder
      `Brain` marker type in `core/brain/mod.rs` with no rules, no scoring and
      no protected paths, so phase 1 fills in a body without changing any
      module signature.
- [ ] 2026-08-06 (phase 0): `FindingKind` variants are not enumerated anywhere
      in the specs — defined the minimum set implied by the module specs
      (`File`, `Directory`, `RecycleBinItem`, `RegistryEntry`, `ScheduledTask`,
      `Other`); expect phases 1–3 to extend it.
- [ ] 2026-08-06 (phase 0): the MIT `LICENSE` needs a copyright holder but no
      name/entity has been chosen (see `PLAN.md` §7, the product name is still
      a codename) — used "Cleanify contributors"; update when the project name
      and owner are final.
- [ ] 2026-08-06 (phase 0): the CI badge in `README.md` needs a GitHub
      owner/repo slug, which does not exist yet — used a `OWNER/cleanify`
      placeholder; the workflow itself is unverified until a remote exists.
- [ ] 2026-08-06 (phase 0): `CONVENTIONS.md` requires user-facing strings to go
      through `t()`, but module display names and descriptions come from Rust
      `ModuleManifest`s — the UI looks up `module.<id>.name` /
      `module.<id>.description` in the dictionary and falls back to the English
      string supplied by Rust, so a Turkish dictionary stays a drop-in.
