# Decisions needed

Open scope questions and contradictions found while implementing. Format:

`- [ ] YYYY-MM-DD (phase N): question — safest interpretation taken`

---

- [x] 2026-08-06 (phase 0): `ARCHITECTURE.md` §1 says `main.rs` holds the Tauri
      builder and command registration, but the Tauri v2 template puts `run()`
      in `lib.rs` (required for the `mobile_entry_point` attribute and for unit
      tests to see the crate) — kept the template layout: `main.rs` is a
      one-line shim calling `winknife_lib::run()`, and the builder plus
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
      a codename) — used "WinKnife contributors"; update when the project name
      and owner are final.
- [ ] 2026-08-06 (phase 0): the CI badge in `README.md` needs a GitHub
      owner/repo slug, which does not exist yet — used a `OWNER/winknife`
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

- [ ] 2026-08-06 (phase 1 remediation): the security review's fix for M7 says a
      link inside a quarantined directory should be "skipped with a per-item
      Skipped report", but a cross-volume move removes the source after copying
      — so skipping a link and then removing the source destroys it, which
      CLAUDE.md forbids outright. Took the safest reading: a directory
      containing a link is **refused** for a cross-volume move, the original is
      left untouched, and the error names the links. Same-volume moves are
      unaffected (a rename carries links intact) and phase 1 never proposes a
      directory, so nothing reachable from the UI changes. Revisit if a later
      phase wants directory cleaning across volumes — recreating links needs
      `SeCreateSymbolicLinkPrivilege`, which this app does not ask for.
- [ ] 2026-08-06 (phase 1 remediation): the M8 profile-containment check cannot
      be evaluated when `%USERPROFILE%` is unset, which is the case for the
      fixture environments the tests build. Took the safest interpretation that
      does not break them: when the profile root is unknown the containment
      half is skipped and the parse-time shape half (drive-absolute, UNC,
      verbatim) still applies. `%USERPROFILE%` is always set on a real Windows
      install, so the real app always gets both halves.
- [ ] 2026-08-06 (phase 1 remediation): M4 says the size-cap loop should "break
      when an iteration frees nothing". A zero-byte item at the head of the
      queue therefore stops the cap from doing anything at all until retention
      removes it. Took that reading deliberately — under-purging is the
      harmless direction, and the alternative (the old behaviour) purged the
      entire quarantine to reclaim nothing. Revisit if zero-byte items turn out
      to be common enough to wedge the cap in practice.
- [ ] 2026-08-06 (phase 1 remediation): `QuarantineStats.unreadableRows` and
      `PurgeReport.affectedBatches` are produced, typed and carried to the
      frontend but **not shown anywhere**. Surfacing them is a quarantine-drawer
      change and the remediation brief did not ask for one; whoever owns the
      drawer should decide how a corrupt ledger row and a partially-evicted
      batch ought to read.

Open items awaiting the producer (not blocking implementation):
- LICENSE copyright holder (currently "WinKnife contributors") — decide with
  the final product name.
  **RESOLVED (producer, 2026-08-06): name is WinKnife; repo is armadavalor/WinKnife.**
- GitHub owner/repo slug for the CI badge — decide when the remote is created;
  CI stays unverified until first push.
  **RESOLVED (producer, 2026-08-06): name is WinKnife; repo is armadavalor/WinKnife.**
- [ ] 2026-08-06 (phase 1 remediation r2): the C2 allow-list means the gate only
      accepts `X:\...`. A future phase that wants to clean a mapped network
      drive or a genuinely remote share will have to widen `gate_path`
      deliberately rather than by accident. Took the narrow reading: phase 1
      packs are local-drive only, and "refused as unjudgeable" is the safe
      answer for everything else.
- [ ] 2026-08-06 (phase 1 remediation r2): `shell:RecycleBinFolder` is now
      reported as unjudgeable by `reason_for`, because it is not a filesystem
      path. Nothing breaks — `scan` routes the recycle-bin pack through the
      shell API before `plan_walks` — but if a later phase gives a pack another
      shell-namespace root it will need an explicit non-filesystem branch
      rather than a walk root.
- [ ] 2026-08-06 (phase 1 remediation r2): supersedes the round-1 note on M4.
      The size cap now *skips* zero-byte items instead of stopping at them, so
      the cap can no longer be wedged by a trickle of empty files (H8). The
      under-purge is now bounded to exactly the bytes zero-byte items hold,
      which is zero.
- [ ] 2026-08-06 (phase 1 remediation r2): `WINKNIFE_ALLOW_SKIP_LINK_TESTS=1`
      turns six link/alias-dependent safety tests into visible skips. The
      development machine has no Developer Mode, so the local verification run
      used it; CI must not, and `ci.yml` fails if it is set. Whoever first runs
      CI should confirm all six actually execute there.
- [ ] 2026-08-06 (first CI run): the five link-dependent safety tests need a
      *symlink*, which requires Developer Mode or elevation. A directory
      **junction** (`mklink /J`) is a reparse point too, `canonicalize` resolves
      it identically, and any standard user can create one — a junction-based
      companion would let the two protected-path link tests assert on every
      machine instead of skipping. Not done here: creating a junction from Rust
      means either shelling out to `cmd.exe` from a unit test or hand-rolling
      `FSCTL_SET_REPARSE_POINT`, and neither is in this brief. Junctions were
      used as a throwaway probe to verify this fix and are not committed.
- [ ] 2026-08-06 (first CI run): `Quarantine::destroy` and `take` guard with
      `quarantine_path.starts_with(&self.root)` on raw text. If the quarantine
      root's spelling ever changes between runs (a short-form `%LOCALAPPDATA%`
      on one run, long-form on another), that guard fails closed — it refuses to
      purge rather than purging the wrong thing — so it is safe as it stands.
      Worth revisiting when retention is exercised in anger.

---

## Architect resolutions — 2026-08-06, before phase 2

Every open item above is answered here. Items marked **ACCEPTED** need no further
work: the interpretation taken is the one we want, and the note stays as the
record of why. Nothing above is left for the implementer to re-decide.

**Phase 0**

- LICENSE holder and CI badge slug — **CLOSED by the producer**: the name is
  WinKnife, the repo is `armadavalor/WinKnife`, and CI has now run green there.

**Phase 1**

- Purge deletes the ledger row, `tracing` keeps the audit trail — **ACCEPTED.**
  A `purged_at` column would only serve a purge-history UI nobody has asked for;
  add the migration if and when the drawer needs it.
- Recycle bin as a pack with the `shell:RecycleBinFolder` sentinel — **ACCEPTED.**
  It keeps one code path for "things the cleaner offers" without pretending a
  shell object is a file. A second shell-namespace root would need an explicit
  non-filesystem branch; note that in the module spec when it happens.
- Cleaner walks directories but never proposes one — **ACCEPTED for V1.** Empty
  leftover folders are the `empty-folder-hunter` backlog module's job, not a
  quiet widening of the cleaner's blast radius.
- `score.base` parsed but ignored — **THIS IS PHASE 2's TASK 1.** Packs written
  during phase 1 stay valid; the scoring pipeline now activates the field. Bucket
  thresholds 85/40 and `max_bucket` ceilings per ARCHITECTURE §4.
- Notifications carry no cancel button on Windows 10 — **ACCEPTED.** Tray and
  route are the cancel paths; revisit only if the plugin gains action support.
- Unset `%VAR%` reported in `skipped_rules` rather than dropped — **ACCEPTED,**
  and it is the pattern to follow: a partial scan must say it was partial.

**Phase 1 remediation**

- A directory containing a link is refused for a cross-volume move — **ACCEPTED.**
  Refusing beats a "skip" that deletes the source behind it. Reversibility wins
  over convenience every time.
- Profile-containment check skipped when `%USERPROFILE%` is unset — **ACCEPTED**
  (shape checks still apply; the variable is always set on a real install).
- The zero-byte size-cap note is **SUPERSEDED** by the r2 fix: the cap now skips
  zero-byte items instead of stopping at them, so it can no longer be wedged.
- `QuarantineStats.unreadableRows` / `PurgeReport.affectedBatches` produced but
  not displayed — **DEFERRED TO PHASE 3** (UI polish owns the quarantine drawer).
  Both must surface then: a corrupt ledger row and a partially-evicted batch are
  exactly the things an honest tool admits to. Do not surface them in phase 2.
- `gate_path` accepts local drive letters only — **ACCEPTED AS A PRODUCT
  DECISION.** V1 cleans local drives. Mapped and remote shares are refused as
  unjudgeable, and any future phase that wants them must widen the gate on
  purpose, with its own review.
- `WINKNIFE_ALLOW_SKIP_LINK_TESTS` — **CONFIRMED.** CI run 31098182451 executed
  all six link tests for real and passed; `ci.yml` fails if the opt-out is set.
- Junction-based companions for the five symlink tests — **ACCEPTED, do it in
  phase 2.** `mklink /J` needs no privilege, so these safety tests would assert
  on every developer's machine instead of only in CI. Shelling out to `cmd.exe`
  from a test helper is acceptable for this; keep it in `core/test_support.rs`.
  This is worth real effort: a junction probe is what proved the last two bugs.
- `destroy`/`take` raw-text root guard — **CLOSED by round 4.** Both now compare
  through `core::path_compare` and reject `Component::ParentDir` outright.

**Standing rule from this phase, binding on phase 2 and after:** any new
path-against-path comparison goes through `core::path_compare`. Three separate
bugs in this phase were the same defect — two spellings of one path compared as
raw text. If a comparison must stay lexical, say why in a comment at the site.

---

## Open items raised during phase 2

- [ ] 2026-08-06 (phase 2): **destructive-action escalation.** Module spec 02
      lists `windows-update-leftovers` targeting
      `%WINDIR%\SoftwareDistribution\Download` at NeedsReview, but `%WINDIR%` is
      a tree-scoped protected path, CLAUDE.md says a protected path can never
      receive a Safe rating *or* a delete action, and RULES-FORMAT.md says "a
      rule matching a protected path is a test failure, not a runtime decision".
      All three cannot hold at once. Safest interpretation taken: the pack ships
      with `action: info_only` + `max_bucket: do_not_delete` and
      `enabled_by_default: false`, so it can report the location and never
      propose anything about it, and an inert pack does not put a permanent
      "not walked, it is protected" line in front of every user.
      `no_shipped_rule_targets_a_protected_path` was tightened rather than
      relaxed: a rule that names a protected path must be structurally unable to
      propose an action, and the test asks the real brain to prove it refuses a
      file under that root. **This is the destructive-action ambiguity CLAUDE.md
      says to always escalate — the architect should decide whether the pack
      stays informational, is dropped, or whether `%WINDIR%` gains a documented
      carve-out.**
- [ ] 2026-08-06 (phase 2): module spec 02 also names `%WINDIR%\*.log` (in
      `crash-dumps-logs`) and `%WINDIR%\Downloaded Program Files` (in
      `installer-leftovers`). Both are inside the same protected tree. Rather
      than ship two more inert rules, those two locations are simply not
      targeted; the packs cover their profile-scoped locations instead. Revisit
      with the decision above.
- [ ] 2026-08-06 (phase 2): the age heuristic reads two clocks rather than the
      one ARCHITECTURE section 4 names ("last-access age"). `SystemLockProbe`
      opens every candidate, so on a volume with last-access updates enabled the
      first scan bumps the access time of everything it examined and the second
      scan of the day would find nothing safe. Age *bonuses* therefore need both
      the access and modification clocks to be old, and the freshness *penalty*
      is keyed on the modification clock alone. Nothing in the spec forbids it,
      but it is a deliberate reading of one sentence as two signals.
- [ ] 2026-08-06 (phase 2): a duplicate's confidence is a flat 75 rather than a
      `brain::score` result. Module spec 03 gives duplicates no `score.base`,
      because a blake3 match is a fact about bytes and not a heuristic — the
      uncertainty is entirely about which copy a person wants. The bucket is
      still NeedsReview minimum and the gates still apply on top.
- [ ] 2026-08-06 (phase 2): `cancel_scan` was added to reach the cancellation
      token phase 1 shipped without a caller (ARCHITECTURE section 3 lists the
      command). It is registered per scan id in `core::state::CancelRegistry`.
      The duplicates route can therefore stop a scan; the cleaner route does not
      yet show a stop button, because a pack walk is seconds rather than minutes
      and the brief did not ask for one.
- [ ] 2026-08-06 (phase 2): a duplicate scan holds its whole candidate list in
      memory (path, timestamps and file id per file). At 10k files that is
      trivial; at a multi-million-file root it would not be. The size floor keeps
      it bounded in practice, and streaming the staged pipeline would be a
      rewrite rather than a tweak. Revisit if anyone points it at a whole drive.

---

## Architect resolutions — phase 2

**The `%WINDIR%` escalation: the pack stays informational. No carve-out, ever.**

The implementer read the three documents correctly — they did contradict, and
raising it rather than improvising was the right call. The permanent answer:

- **`%WINDIR%` gets no exception.** A "documented carve-out" is how a protected
  path list erodes: the mechanism outlives the justification, and the next
  contributor argues their directory is just as safe. This phase was bitten four
  times by protected-path defects; we are not adding a door.
- **Informational is not a workaround, it is the design.** `PLAN.md` §3.1 defines
  the red bucket as "information only, no button — this is the honesty feature",
  and `windows-update-leftovers` is the first real case of it. Keep it shipping,
  keep it inert, but **make the `reason` earn its place**: it must name the size
  found *and* point at the safe alternative (Windows' own Disk Cleanup /
  Settings → System → Storage). "There is 4 GB here, I will not touch it, here is
  what will" is a better answer than silence, and it is exactly the trust the
  README will claim.
- Leaving `enabled_by_default: false` is right — an inert pack should not put a
  permanent "not walked" line in front of every user who never asked.
- Tightening `no_shipped_rule_targets_a_protected_path` instead of relaxing it
  was the correct instinct and is now the standing pattern: **a rule naming a
  protected path must be structurally unable to propose an action**, proven
  against the real brain, not by inspection.
- `%WINDIR%\*.log` and `%WINDIR%\Downloaded Program Files` **stay unshipped.**
  One informational rule makes the point; three would be noise pretending to be
  coverage.
- **Docs to correct so this cannot come back** (do these in the next round):
  `docs/modules/02-cleaner-core.md` lists `windows-update-leftovers` as
  NeedsReview and names the two `%WINDIR%` locations — change the table to
  InfoOnly and delete those two rows, with a line saying why.
  `docs/RULES-FORMAT.md` says "a rule matching a protected path is a test
  failure"; make it precise — a rule that *proposes an action* on a protected
  path is a test failure, an informational rule is permitted and must be
  structurally incapable of acting.

**The access-clock problem is a CLAUDE.md violation and must be fixed properly.**

CLAUDE.md is unambiguous: "All scans are read-only. `scan()` and `analyze()` must
not modify the filesystem." A lock probe that opens every candidate bumps each
file's last-access time on any volume where last-access updates are enabled — the
scan mutates the very signal it is reading, and a second scan of the day finds
nothing safe. Requiring both clocks to be old is a sound defensive reading, but it
treats the symptom.

**Fix the cause:** Windows documents a suppression for exactly this — after
opening the probe handle, call `SetFileTime` with the last-access argument set to
a `FILETIME` of `{0xFFFFFFFF, 0xFFFFFFFF}`, which disables access-time updates for
the remaining lifetime of that handle. Do that in `SystemLockProbe`, and add a
test proving a probe leaves the access time untouched. Keep the two-clock reading
as defence in depth. Modern Windows disables last-access updates by default, so
this is rare rather than theoretical — which is precisely why it would have been
found late and blamed on something else.

**Accepted as implemented, no change needed:**

- **Duplicate confidence is a flat 75, not a `brain::score` result** — correct. A
  blake3 match is a fact about bytes; the uncertainty is which copy a person
  wants, which no heuristic score should pretend to quantify. NeedsReview
  minimum and the gates on top are the right shape.
- **`cancel_scan` reaching the token phase 1 shipped without a caller** — good
  catch, keep it. No stop button on the cleaner route: a pack walk is seconds.
- **The duplicate candidate list is held in memory** — fine for V1 behind the
  size floor. If anyone points it at a whole drive we stream it; that belongs in
  the backlog, not in this phase.

---

## Open items raised during the phase 2 review remediation

- [ ] 2026-08-06 (phase 2 review, H10): the executor's new justification gate
      re-checks **what the finding recorded**. A finding whose `meta` carries
      neither a duplicate hash nor a pack/rule id is left to the first three
      gates exactly as before, because there is no recorded reasoning to
      re-derive. Every shipped module records one, so nothing reachable from the
      UI is unchecked — but the guarantee is "modules that record a
      justification get it verified", not "every action is verified". Closing
      that gap means making a recorded justification part of the module
      contract (`ToolModule`), which is a phase-3 change and not this brief's.
- [ ] 2026-08-06 (phase 2 review, H10): the gate re-tests the rule's own `match`
      criteria and **not** `modules::cleaner::MIN_QUIET_HOURS`. The quiet period
      is a module-level scan policy rather than a rule criterion, and lifting it
      into `core` to re-check it would put a cleaner constant in the executor.
      Every shipped rule declares `min_age_hours` of 24 or more, which subsumes
      the one-hour quiet period for all of them; a future pack that declares no
      age at all would not get a freshness re-check. Revisit if one ships.
- [ ] 2026-08-06 (phase 2 review, H10): the duplicate re-check re-reads the file
      in full. That is a second read of every *approved* item — not of every
      candidate — and the quarantine move is about to read every byte anyway on
      a cross-volume copy. Sampling was rejected outright: it would re-admit
      exactly the crafted collision `a_crafted_collision_is_not_a_duplicate`
      exists to forbid. If this ever shows up in a profile, the answer is to
      skip the re-read when the file's size *and* modification time are both
      unchanged since the scan — a cheap conservative pre-filter — not a
      partial hash.
- [ ] 2026-08-06 (phase 2 review, A2): the architect asked the
      `windows-update-leftovers` reason to "name the size found". Rule `reason`
      strings are static data and ship to the UI verbatim
      (`docs/RULES-FORMAT.md`), so a literal byte count cannot appear in one
      without adding placeholder interpolation to the rule-pack format — new
      scope, and a format change ripples to every pack and to the schema. Safest
      interpretation taken: the reason names the space *by pointing at the
      figure the row already shows* and names both safe alternatives (Settings →
      System → Storage, and Disk Cleanup). If interpolation is wanted, `{size}`
      in `reason` expanded by the brain is the obvious shape — decide it as a
      format change rather than as a one-pack special case.
- [ ] 2026-08-06 (phase 2 review, M14): the walk-root fold now re-anchors an
      absorbed rule's globs by re-spelling the walked path, which keeps one walk
      per directory. The alternative the review offered — one group per spelling
      — was not taken because it undoes the L5 optimisation on precisely the
      case that motivated it (`%TEMP%` in 8.3 form against a long-form
      `%LOCALAPPDATA%\Temp`, which is the stock layout on this machine).

---

## Open items raised during phase 3

- [ ] 2026-08-06 (phase 3): module spec 04 asks for a "parallel walk" and the
      phase brief for "parallel bottom-up size aggregation". Measured on the
      100 000-file synthetic tree (`src-tauri/tests/treemap_perf.rs`, release,
      warm cache): `jwalk`'s parallel enumeration alone was **64 ms**, the same
      enumeration plus the `fs::symlink_metadata` its `DirEntry::metadata()`
      performs was **3 924 ms**, and a plain sequential `read_dir` walk using the
      directory listing's own metadata was **143 ms**. On Windows
      `symlink_metadata` opens the file; `DirEntry::metadata` answers from the
      `WIN32_FIND_DATA` the listing already returned. So parallelism was worth
      80 ms and the syscall was worth 3 800 ms. `core::scanner` is now a sorted
      depth-first `read_dir` walk — sequential, streaming, deterministic — and
      `jwalk` has been dropped. First paint on the 100k tree went from 8 678 ms
      to **868 ms** against a 3 000 ms budget. The aggregation *is* still
      bottom-up, as one linear depth-ordered sweep. If a future phase points this
      at a spinning disk where concurrency pays for itself, the place to add it
      back is one function, and the benchmark to prove it is already committed.
- [ ] 2026-08-06 (phase 3): the cheap path above has one caveat, found by
      `duplicates::scanning_never_modifies_anything` failing intermittently
      after the rewrite. NTFS keeps a **copy** of each entry's timestamps in its
      parent directory's index and updates it lazily, so `DirEntry::metadata()`
      can report a modification time older than the truth for something that
      changed moments ago. Sizes and attributes are unaffected. That matters
      because the cleaner's age heuristic reads modification times and "older
      than the truth" is the direction that makes a freshly written file look
      abandoned. Safest interpretation taken:
      `WalkOptions::accurate_timestamps` defaults to **true**, so the cleaner and
      the duplicate finder pay one stat per entry and behave exactly as they did
      before the rewrite; only the treemap opts out, because it reads no clocks
      at all and is the only thing that walks a whole drive. The speedup is
      therefore the treemap's, not everyone's — the earlier claim in this file
      was wrong and is corrected here.
- [ ] 2026-08-06 (phase 3): module spec 04's cross-links say the actions route
      "with path pre-filled". That works for **Find duplicates here**
      (`/duplicates?root=…` narrows the scan to that folder) and for **Open in
      Explorer**. It does not map onto the cleaner: the cleaner scans the
      locations its rule packs name, not a folder the user picks, so there is no
      path parameter to pre-fill. Rather than pretend, `/cleaner?from=…` shows a
      line saying where the user came from and that the cleaner looks at known
      junk locations — so the link goes somewhere useful and does not imply a
      filter that is not there. Giving the cleaner an arbitrary-path mode would
      widen its blast radius and is a product decision, not a UI one.
- [ ] 2026-08-06 (phase 3): module spec 05 lists "disk vs SSD" as an input to
      the impact estimate. It is not implemented: detecting whether a volume is
      solid-state means another Win32 surface
      (`IOCTL_STORAGE_QUERY_PROPERTY`), and the tier is derived from the
      program's size and the known-app list instead. Every place the tier appears
      says "estimated" and `impact_reason` names exactly what it was derived
      from, so this is a refinement rather than a correction — but the spec's
      third input is missing and this is the record of it.
- [ ] 2026-08-06 (phase 3): a Startup-folder entry's signature badge reports on
      the `.lnk` file, not on the program the shortcut points at, because
      resolving a shortcut needs `IShellLink` — a fourth COM surface. The badge
      is therefore usually "signature could not be checked" for folder entries,
      which is honest but less useful than it should be. `executable_of` already
      does the work for registry and task entries; a shortcut resolver would slot
      in at the same place.
- [ ] 2026-08-06 (phase 3): long scans run on the async runtime rather than
      `tauri::async_runtime::spawn_blocking`, which ARCHITECTURE section 3 names.
      This matches the shape phases 1 and 2 shipped for the cleaner and the
      duplicate finder, so the treemap follows it rather than introducing a
      second pattern late in a phase. It is defensible on the numbers — a
      100 000-file walk is 0.87 s and cancellation lands in 41 ms — but a
      whole-drive walk on a slow disk is minutes, and it blocks one tokio worker
      for that time. Worth doing properly for all three modules at once rather
      than for one of them now.
