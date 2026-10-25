<!-- durum: aktif -->
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
      name/entity has been chosen (see `docs/PLAN.md` §7, the product name is still
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
- **Informational is not a workaround, it is the design.** `docs/PLAN.md` §3.1 defines
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
- [ ] 2026-08-06 (phase 3 review): `core::ledger` admits exactly two orders for a
      move of user data — `Claim` (row committed first) and `Retire` (row dropped
      after). The quarantine's restore stays a `Claim` because it *marks*
      `restored_at` rather than deleting the row, so the file never stops being
      findable (the M2 argument still holds); the startup restore is a `Retire`
      because its row really is deleted. A third shape — "amend the row to say
      the move is in flight" — would close the crash window on both without a
      `reconcile` pass, but it needs a column on both tables and therefore a
      migration. Safest interpretation taken: `reconcile` reads the filesystem
      instead, and only a clean `NotFound` counts as evidence.
- [ ] 2026-08-06 (phase 3 review): `SetFileTime`'s `0xFFFFFFFF` access-time
      opt-out needs `FILE_WRITE_ATTRIBUTES` on the handle, which is not obvious
      and fails silently as `ACCESS_DENIED`. `lock.rs` already asks for it;
      `signature.rs` did not, and on a volume where last-access updates are live
      the suppression was rejected outright. Both now ask for it and fall back to
      plain read when an ACL refuses. Worth a note because any future reader of a
      user's file has to know: **read access alone is not enough to promise the
      clock will not move.**
- [ ] 2026-08-06 (phase 3 review): the treemap's arena no longer asks
      `path_compare::same_path`/`is_inside` per directory — it uses
      `Path::strip_prefix` against the scan root, which is exact for paths the
      walk built by joining onto that root and costs no syscall. A path the walk
      could not have produced (outside the root, or containing `.`/`..`) gets a
      parentless node rather than a guess. This is a deliberate narrowing of what
      the arena resolves, justified by the walk's own guarantee that it never
      leaves its root; if a future caller feeds `treemap::scan` paths from
      somewhere else, that assumption is the one to re-check.

- [ ] 2026-09-13 (phase 4): User prioritizes polish, release, then Graphify. Release engineering is prepared first; optional LLM adapters, keyring and privacy UI remain unimplemented. No claim of Phase 4 acceptance until its full scope and fresh-VM checks are satisfied.

## Phase 4 release audit — 2026-10-03

A read-only audit of the hard rules before the first release (raw deletes,
network, read-only scans, protected paths, reasons) found two violations and
three items needing review. The fixes are logged in CHANGELOG-DEV; what is
recorded here is the decisions they rest on, and the one decision that is
not ours to take.

- [x] 2026-10-03 (phase 4, ESCALATED — destructive; resolved 2026-10-11 by
      the owner, option a): **emptying the recycle bin is the one documented
      exception to the first hard rule.** `SHEmptyRecycleBinW`
      (`core/platform/recycle_bin.rs`) deletes permanently and does not go
      through quarantine. The phase 1 resolution accepted it and spec 02 calls
      it "the one non-quarantine action", but CLAUDE.md was never amended, so
      the audit flagged it. Before the fix the rule (`rules/recycle-bin.json`,
      score 95, `max_bucket: safe`) made the row green and the cleaner
      pre-selected every green row, so **the recycle bin started ticked**; the
      confirm dialog added a danger warning and the backend refused unless
      `confirm_empty_recycle_bin` came from that dialog. Options were (a) a
      narrow carve-out and the row never starts ticked, (b) a carve-out as the
      code stood, (c) drop the action. Taken: (a). CLAUDE.md and CONTRIBUTING
      now name the exception; `initialSelection` (`stores/selection.ts`, with
      tests) keeps the row unticked after a scan, so emptying it takes a tick
      the user makes plus the separate warning. The row stays green: the
      rating says the contents are safe to lose, the selection says WinKnife
      will not choose it for you.
- [x] 2026-10-03 (phase 4): **the LLM layer is deferred to v0.2.0.**
      v0.1.0 ships with no network client at all. Its opt-in, the
      exact-payload preview and the keyring move together to the next minor
      version, so the first release can make the simplest privacy promise
      there is. The README must not mention LLM features until then, and phase
      4 acceptance is claimed against this narrowed scope only.
- [x] 2026-10-03 (phase 4): **log retention is the newest 14 daily files.**
      Logs carry the local paths of cleaned items, and nothing ever removed
      them. `core/logging.rs` now asks tracing-appender for `max_log_files(14)`.
      It counts files, not days: when the log opens and at each daily rotation
      it deletes the oldest `winknife.log*` files in the app's own log
      directory. Two consequences worth knowing: (1) the phase 1 resolution
      "purge deletes the ledger row, `tracing` keeps the audit trail" now has
      a horizon: a purge line lives about 14 days of use after the purge,
      which still outlives the 7-day default quarantine retention; (2) the old
      `rolling::daily` panicked when the log file could not be opened; the
      builder hands the error back, and the app starts without a file log.
      The deletion happens inside the dependency, on app-owned files, so it is
      outside the hard rule; recorded here so nobody has to rediscover that.
- [x] 2026-10-03 (phase 4): **a startup entry is rated Safe only when its
      program clears the cleaner's protected-path gate.** The known-app list
      (`startup-known.json`) matches on the command line alone, so an entry
      running `C:\Program Files (x86)\Steam\steam.exe` came out green.
      `startup_rating` (`commands.rs`) now asks `Brain::path_refusal`, the
      same gate the cleaner uses. Safe needs three things: a safe-to-disable
      verdict, a resolved program, and no refusal. Anything else is
      NeedsReview, and the reason says why ("It is not rated safe: …").
      Consequence worth knowing: most third-party programs live under
      `Program Files`, so few startup entries are green any more; programs
      installed per user under `AppData` still can be. Turning an entry off
      is unchanged: still offered, still reversible, nothing removed. Notes
      from the fix, none blocking:
      (1) a `target` naming a missing file passes the gate unless the lexical
      list refuses it. Only hand-built entries can carry one, because
      `executable_of` returns existing files only;
      (2) the gate's wording ("protected location, not cleanable: … use the
      app's own uninstaller instead") was written for the cleaner and reads
      oddly inside a startup reason. Reword when the startup reason reaches
      the UI. **Resolved 2026-10-24 (#4):** the startup reason names the
      protected place from the list's own entry and drops the cleaner's
      advice;
      (3) the startup page still shows the known-app label "Safe to turn off"
      (grey, from `entry.verdict`) beside the known-app note, never the
      rating or its reason. That is no green rating, and the note explains
      the label, so it is not a hard-rule breach; but for a protected program
      the page and the backend now disagree. Open for polish: show the rating
      and reason, or soften the label. **Resolved 2026-10-24 (#4):** an
      entry Rust rated shows the rating (green "Safe to turn off" or yellow
      "Review before turning off") and its reason under "Why this rating?";
      entries without a rating keep the grey known-app label;
      (4) QA exercised the gate only through unit tests: the protected
      entries on the QA machine (Epic, Spotify) were already disabled, so no
      live entry flipped from green to review.
- [x] 2026-10-03 (phase 4): **the webview has a content security policy, and
      the unused opener plugin is gone.** `tauri.conf.json` shipped
      `"csp": null`, and the opener plugin was registered with
      `opener:default` although nothing used it. The policy now is
      `default-src 'self'; script-src 'self'; style-src 'self'
      'unsafe-inline'; img-src 'self'; font-src 'self'; connect-src ipc:
      http://ipc.localhost; object-src 'none'; base-uri 'self'; form-action
      'none'; frame-src 'none'`. The choices behind it:
      `style-src` keeps `'unsafe-inline'` because Svelte writes `style=`
      attributes (treemap tiles, bars); Tauri adds hashes only for inline
      `<script>`/`<style>` elements and the built HTML has no `<style>`
      element, so nothing switches `'unsafe-inline'` off. Scripts get no such
      allowance: Tauri hashes the one inline SvelteKit bootstrap at build
      time. `img-src` drops `data:`, which nothing uses today; add it back
      deliberately if icons ever arrive as data URLs. `connect-src` is
      Tauri's IPC only, which matches "no network". No `devCsp`: in Tauri
      2.11.5 the Windows dev page gets no CSP at all, so CSP checks mean
      something only in a release or QA build. The opener left Rust
      (`lib.rs`, `Cargo.toml`), the capability file and npm; `Cargo.lock`
      lost `tauri-plugin-opener`, `open`, `is-wsl` and `is-docker` and
      nothing else. QA (release build, `com.winknife.qa` identity): 7 routes,
      35 IPC calls answered, 0 console errors, 0 CSP violations; an image and
      a fetch to an outside host were both blocked. `release-ipc.json` has no
      command list, so `list_modules` stood in as the IPC probe.
- [x] 2026-10-03 (phase 4), revised 2026-10-11 after two adversarial
      reviews: **a cross-volume move pins the source and deletes through
      the pins.** Every folder, the top one included, is opened as itself
      (links are never followed) and held without sharing delete until its
      own delete, so it cannot be swapped for a junction mid-move. A folder
      that is a link or reparse point is refused, and every file and folder
      must resolve inside the top folder's real path (whole components,
      case-insensitive). Each file is pinned against writes. Its main data
      stream and its Zone.Identifier stream are copied and checked byte for
      byte, and so is its list of named streams; a file or folder with any
      other alternate data stream is refused before anything is created.
      Read-only, hidden, system, archive and not-indexed attributes come
      across and are checked. Deletes go through the pins: after each
      delete is marked, the streams are listed again, and if they changed
      the delete is taken back and the file stays at the source (if it is
      the first file, the whole move is undone). A probe on NTFS confirmed
      the take-back clears the pending delete and that a marked file cannot
      be reopened or gain a stream. The take-back is verified: the same
      handle is asked whether the delete is still pending. If the take-back
      fails or cannot be confirmed, the source is treated as gone: the
      verified copy stays, the move goes ahead, and the user is told that
      the stream which arrived could not be kept.
      Before the copy is undone, the source's pin is closed and the source
      is checked to still be there; if it went, the copy stays. Refused
      before anything is created, with a reason: a file or folder encrypted
      with Windows EFS ("moving it across drives would store it
      unencrypted"; a rename on one volume still works and keeps the
      encryption), and a file with more than one name ("hard link"; before
      this, a tree holding two names of one file was moved as two separate
      copies). `open_to_delete` and `open_dir_to_delete` are visible to
      `core` only, and the guard test rejects them, delete-on-close,
      `NtSetInformationFile`, the ANSI deletes, the shell's removal and
      `MOVEFILE_REPLACE_EXISTING` outside `src/core/quarantine.rs` (by exact
      path). Once any source file is gone the move counts as done and
      keeps its ledger row; leftovers are reported to the user. Paths over MAX_PATH work.
      Before this patch the copy already lost alternate streams and
      hidden/system attributes; that loss was not introduced here. Known
      limits: streams other than Zone.Identifier are refused, not carried.
      EFS-encrypted and hard-linked files are refused, not moved, across
      drives. Creation time, folder times and ACLs are not kept. For a lone
      file, an ancestor junction retargeted between pin and move is not
      detected (the moved file is still copied and verified). Read-only
      files on FAT stay behind. The destination is not pinned between
      verification and the source delete. Test note: the EFS test needs an
      EFS-capable volume (NTFS, Windows Pro or above; it reports itself
      skipped otherwise). On first run it makes Windows create the user's
      own EFS certificate, and Windows may offer to back up the key.
      `cipher` only runs on paths inside the test's own temporary
      directory, which the test removes.

      Follow-up review (2026-10-12). What a move that did happen left
      undone now reaches the user: the take and restore results carry
      warnings (path and plain-English reason) shown under the cleaner,
      duplicates, startup and quarantine-drawer results. Covered: part of
      a folder left at its old place, a source gone although its delete
      was refused (only the moved copy remains), and a stream that arrived
      during the move and could not be kept. The ledger schema is
      unchanged; the warnings are optional fields on the result types. A
      late stream whose take-back fails is no longer counted as moved. The
      move still reports success with a warning, because the source is
      already gone and an error would drop the ledger row of the only copy
      (security review C2). After a refused delete, the source counts as
      gone only when its name is not found or holds another file. When its
      identity cannot be read, both are kept and the move succeeds with a
      "both kept" warning naming the old place, so the ledger row of the
      moved copy survives (round 2, 2026-10-12; it first failed, which
      left a quarantined copy with no ledger row). That was chosen over
      guessing.

      Decided 2026-10-12 (user decision, option b): an EFS-encrypted item
      whose restore is refused across drives stays held and is never
      removed by the automatic purge. Implementation:
      - Migration 004 adds a nullable `restore_refused_at` column to
        `quarantine`. It is additive, so a version-3 database loads and its
        rows read as unmarked.
      - `restore_item` sets the marker when `move_out` fails with the
        typed encrypted refusal, after the ledger has put the row back to
        held. A failure to set it is logged, not returned.
      - `purge_expired` skips marked items. `enforce_size_cap` never evicts
        them. *(Superseded in round 4, below: it no longer counts them
        towards the total either.)*
      - The only removal path is the user's: `Quarantine::purge_item`,
        exposed as the `quarantine_purge_item` command and the drawer's
        Remove button behind a confirmation that says it is permanent.
        The drawer labels such an item "restore refused: encrypted; kept
        until you remove it".
      Scope choice, open for review: `purge_item` refuses an item without
      the marker, so it does not become a second delete button for
      ordinary held items (those stay with their retention window).
      *(Resolved 2026-10-25, K-020: reversed by the user. Any held item, a
      selection or a whole batch can now be removed for good through
      `quarantine_purge_items`, behind the drawer's "cannot be undone"
      dialog with item count and size; the backend refuses the call
      without `confirmed: true`. Deletion still goes only through
      `destroy()`. The marker now only shields items from the automatic
      purge and the size cap.)* The
      marker is not cleared by a later successful restore; once restored,
      the row is no longer held and the marker has no effect.

      Follow-up review, round 3 (2026-10-12):
      - *(Superseded by round 4, below.)* Size cap. When the marked items
        alone are at or over the cap,
        `enforce_size_cap` removes nothing for size. Before this, it evicted
        every unmarked item and still ended up over the cap, so each later
        clean was made irreversible at the next start. The retention window
        still applies to unmarked items. `QuarantineStats` now carries
        `refused_bytes` and `cap_bytes`, and the drawer explains in plain
        English why nothing is being removed for size. When the marked
        items are under the cap, eviction works as before.
      - When the marker cannot be written (a database error, or no held row
        matched), the refusal message no longer promises to keep the item.
        It says the automatic purge may still remove the item when its
        retention ends (with the date) or when the quarantine is over its
        size limit, and tells the user to decrypt and restore before then.
        This option was chosen over returning a different error because
        the restore is refused either way. The user needs to see the
        refusal and how to act on it, not an internal database failure.
      - Remove in the drawer shows the item's quarantine path, and the
        confirmation does too. A Remove that fails has its own message.
        The purge's "retried on the next purge" would be wrong here,
        because the automatic purge never touches a marked item.

      Follow-up review, round 4 (2026-10-12). *(Closed 2026-10-12, phase 5
      task 0: accepted as built, no code change. Marked items stay outside
      the size cap.)*
      - Marked items are now outside the size cap. `enforce_size_cap`
        measures unmarked bytes only (`over_by = unmarked - cap`) and still
        never evicts a marked item. Round 3's rule (remove nothing for size
        once the marked items fill the cap) left a cliff just below it: with
        marked items at 3.99 GiB of a 4 GiB cap, any clean bigger than the
        last 10 MB evicted older unmarked items at the next start, and one
        byte more stopped the cap altogether. Consequence to review: the
        quarantine folder can now hold the cap plus whatever the marked
        items hold, without limit, until the user removes them. The
        retention window still applies to unmarked items.
      - `QuarantineStats` also carries `refused_items`. The drawer explains
        whenever any marked items are held, not only once they fill the
        cap: how many, how much they use, that they are not counted against
        the size limit, and that only the user removes them.
      - The drawer's Remove failure message now says "still held, never
        auto-purged" only when the file could not be deleted. When the
        backend refuses (the item is no longer held, or it is not marked,
        from a stale list), it says so and asks the user to refresh. Any
        other error is shown as it came.

## Open items raised in the follow-up review, round 3 (2026-10-12)

- [x] **Startup stash: the "both kept" (Unknown) arm leaves the original
  shortcut in the Startup folder.** This happens on a cross-volume stash
  whose source delete was refused and whose identity then cannot be read.
  The move succeeds with a "both kept" warning, so the ledger row of the
  stored copy survives. The original `.lnk` also stays in the Startup
  folder, though, so it still runs at logon. Meanwhile the page shows the
  entry as disabled, with the warning. Re-enable then tries to move the
  stored copy back onto a name the original still occupies, and that fails
  as Occupied: the move primitive never overwrites. Nothing is lost, and
  the warning names the old place, so the user can delete the original by
  hand. Still, "disabled" is not true until they do. Options:
  (a) leave as is; the warning is the signal.
  (b) show such an entry as "disabled, but its shortcut is still in the
      Startup folder", and make re-enable drop the stored copy when the
      original at that name has the same contents.
  (c) refuse the stash in this arm instead. That was rejected in round 2
      because it dropped the ledger row of a copy that may be the only one.
  Not changed in round 3. Option (b) touches re-enable's delete rules, so
  it needs an architect decision.
  **Closed 2026-10-12 (phase 5 task 0): option (b).** The entry is listed
  once, as off, with `leftoverShortcut` set and the label "Off — but its
  shortcut is still in the Startup folder" plus a reason naming the path.
  Re-enable compares the two files through `content_hash::full_hash`: equal
  → the stored copy goes to the quarantine (never `fs::remove_*`) and the row
  is forgotten; different, or unreadable → the Occupied refusal, saying which,
  with both files kept.
- [x] **A database from a newer build.** `run_migrations` now refuses a
  database whose `user_version` is newer than the newest migration this
  build knows (`CoreError::SchemaTooNew`, code `schema_too_new`). An older
  build would otherwise run with a schema it does not understand and could,
  for example, purge items that a newer build promised to keep. Startup
  fails in setup, and a native error box (`MessageBoxW`, only in a build
  armed for real OS effects) tells the user to install the newer version
  again. Round 4: `manage_state` now logs the `db::open` error itself,
  while the log file's appender guard is still alive, so the refusal
  reaches the log file. The setup's own log line afterwards can still miss
  it, because the guard is dropped when `manage_state` returns; it only
  repeats what is already recorded. Downgrading a database is not
  supported.
  **Closed 2026-10-12 (phase 5 task 0).** The appender guard now goes into
  managed state right after `logging::init`, before `db::open`, so every
  later setup error reaches the log file
  (`tests/log_guard_survives_setup_error.rs`). Downgrade mitigation: won't
  fix; the `schema_too_new` refusal stays the whole answer.

## Open items raised during phase 5

- [x] 2026-10-12 (phase 5): `quarantine_purge_item` on an unmarked item now
  returns its own code, `quarantine_item_not_marked`, instead of sharing
  `unknown_quarantine_item`; the drawer maps it to its own message ("its
  restore was never refused … stays held until its retention window ends").
  *(Superseded 2026-10-25, K-020: `quarantine_purge_items` removes any held
  item after confirmation, so the code and its check are gone.)*
- [ ] 2026-10-12 (phase 5): the brief asks for EN and TR strings, but the app
  had no Turkish dictionary. A partial `tr` dictionary was added holding the
  phase 5 keys only (missing keys fall back to English), with no locale
  switcher — safest interpretation taken; a switcher and a full translation
  are out of this phase's scope.
- [ ] 2026-10-12 (phase 5): a held startup copy quarantined by re-enable
  (issue #21) restores, if the user asks, to its old place in WinKnife's
  `startup-disabled` folder with no ledger row, so it is not listed as a
  startup entry. Nothing is lost and the original shortcut is untouched;
  left as is — safest interpretation taken.
- [ ] 2026-10-12 (phase 5): found in passing, not fixed (out of scope):
  `StartupEntry` serializes camelCase (`osOwned`, `osReason`,
  `signatureNote`, `impactReason`), but `src/lib/types.ts` and
  `startup.svelte.ts` read the snake_case names, so the UI sees them as
  undefined (an OS-owned entry's toggle looks enabled; the backend still
  refuses it). The new `leftoverShortcut` field uses the real wire name —
  safest interpretation taken; the mirror needs a separate fix.
- [ ] 2026-10-12 (phase 5): the restore-point spec lists two commands, but
  "offer `SystemPropertiesProtection.exe`, opened only on button click" needs a
  third. Added `restore_point_open_settings` (opens Windows' own dialog through
  `ShellExecuteExW`, changes nothing) — safest interpretation taken.
- [ ] 2026-10-12 (phase 5): "System Protection looks enabled for the system
  drive" is read from HKLM registry only (policy `DisableSR`, then
  `SystemRestore\RPSessionInterval`), not from per-volume shadow-storage
  config, which needs admin/WMI. Missing values report "unknown" — safest
  interpretation taken.
- [ ] 2026-10-12 (phase 5): the helper's failure exit code carries only the
  low 16 bits of the Win32/HRESULT code (`0x2000_0000 | code`), so it never
  collides with the success codes or overflows `i32`. An HRESULT failure is
  therefore shown truncated — safest interpretation taken.
- [ ] 2026-10-12 (phase 5): two result states beyond the spec's five:
  "may have skipped" (spec text, when WMI cannot confirm) and "timed out"
  (helper still running after 5 minutes; it is left alone, never killed).
  A second create while one is running is refused — safest interpretation
  taken.
- [ ] 2026-10-12 (phase 5): ProgramData is a protected tree, so the `program-data` NeedsReview rule in `rules/empty-folders.json` is never reached (protected trees are skipped, not reported) — kept as is, harmless.
- [ ] 2026-10-12 (phase 5): Brain and executor skip the lock probe for directories (an empty folder has no handle to probe); the execute-time emptiness re-check replaces it — kept the re-check, the safest reading.
- [ ] 2026-10-12 (phase 5): Rule packs are now filtered per module (cleaner loads only `cleaner`, `read_settings` too) so the empty-folders pack cannot leak into the cleaner — added a module filter.
- [ ] 2026-10-12 (phase 5): The `**` fallback and `program-data` rules set `requires_admin: true` only to satisfy the rule-containment validator; it has no runtime meaning here.
- [ ] 2026-10-12 (phase 5): UNC paths match no rule and become DoNotDelete; an 8.3-spelled `%TEMP%` may fall to NeedsReview — both err toward not deleting.
- [ ] 2026-10-12 (phase 5): Invalid scan roots are reported as `CoreError::Io(InvalidInput)` rather than adding an error variant — minimal diff.
- [ ] 2026-10-12 (phase 5): `program-data` is now `info_only` capped at do_not_delete (it sits in a protected tree), and cleaner test `no_shipped_rule_targets_a_protected_path` skips the empty root of the catch-all `**` glob — the only test edit beyond pack filtering.

## Open items raised during phase 5 — file-unlocker (2026-10-12)

- [ ] 2026-10-12 (phase 5): the brief asks for a file/folder pick "via the dialog plugin", but `tauri-plugin-dialog` was not in the project. Added it (Cargo `tauri-plugin-dialog = "2"`, npm `@tauri-apps/plugin-dialog`, capability `dialog:allow-open` only — no save/message permissions, one `.plugin(tauri_plugin_dialog::init())` line in `lib.rs` next to the notification plugin). Drag-drop and a typed path work without it — safest reading: smallest permission set, open dialog only.
- [ ] 2026-10-12 (phase 5): the brief asks for EN + TR keys, but `i18n.ts` had only an English dictionary. Added a partial `tr` dictionary holding only the File Unlocker keys (everything else falls back to English) and widened `Locale` to `"en" | "tr"`; nothing calls `setLocale("tr")` yet and there is no language picker. Other phase 5 workers adding TR keys will touch the same object — expect a trivial merge. The Rust-built `reason` and the `refused` reason text stay English; the UI rebuilds row reasons from `reason_code` so they translate.
- [ ] 2026-10-12 (phase 5): Restart Manager kinds other than window/console/service/explorer/critical (`RmUnknownApp` and anything unrecognised) are treated as info-only with a reason, never closable — safest interpretation of "Ask to close applies ONLY to window and console apps".
- [ ] 2026-10-12 (phase 5): the process start time (a `FILETIME`, ~1.3e17) does not fit a JS number exactly, so it travels as a decimal string and `unlocker_request_close` takes `start_time: String`; a non-numeric value is refused (`invalid_start_time`).

## Open items raised by the QA audit (2026-10-17)

- [ ] 2026-10-17 (QA): long operations that can leave the UI spinning were audited, not fixed (docs/QA-PLAN.md, Audit). Priority to decide: the global DB mutex (#37), duplicates Stop with a stale scan id (#38), no cancel/busy guard on execute/restore/purge (#39), panics leaving promises pending (#40); medium items grouped in #41. Cleaner Stop button and `busy` stuck after a failed `attachProgress` (A3/A4) go with the cleaner scan-hang fix.
- [ ] 2026-10-17 (QA): the e2e job runs only on PRs to main and manual dispatch, not on push to main, because it really cleans and restores on the runner. A nightly 100k-Temp run would catch scale regressions; not added (cost) — decide if wanted.

## Open items raised by the user (2026-10-18)

- [x] 2026-10-18 (user): "offer a direct delete that skips the quarantine, it could make deleting faster". This contradicts CLAUDE.md's hard rule (never permanently delete user files; the recycle bin is the one documented exception) — destructive, so escalated, not implemented. Facts for the decision: the slow 20 GB clean was the lock probe, not the quarantine move (a same-volume rename is ~0.4 ms); after the probe fix a quarantine clean of 85k files should take ~1–2 min. Options: (a) keep quarantine-only; (b) a per-run "delete immediately" opt-in that still moves to quarantine and purges right after, so the executor's three gates and the log stay; (c) true direct delete (needs a new K- decision and a CLAUDE.md change). Recommendation: (a), revisit (b) if the faster clean still feels slow.
  - **Resolved 2026-10-18 (user, K-016):** option (a), quarantine-only. Revisit after the user has tried the faster clean.
- [ ] 2026-10-19 (Polish 8): the brief says animations go off "in a performance mode", but the app has no performance mode. The switch covers the toggle and `prefers-reduced-motion`; a future performance mode should call the same motion store rather than adding a second attribute.

## Open items raised by the QA/QC pass (2026-10-20)

- [x] 2026-10-20 (QA/QC, docs/QA-CHECKLIST.md A9): a restore claims the ledger row (`restored_at` set) before it moves the bytes back (security review M2, so two restores cannot race). If the process is killed between the claim and the move, the row is hidden from the drawer and skipped by purge: the bytes stay in the quarantine folder forever, with no way to restore or free them from the UI. Nothing is lost, but it leaks space and the user cannot see the item. Proposed fix: on startup, clear `restored_at` for rows whose `quarantine_path` still exists, so the item shows as held again. Caveat: after a crash in the middle of a cross-volume copy, the partial copy at the original path stays, and the item shows as held (a later restore then gets a `(restored)` name). Destructive-path change, so escalated, not implemented.
 **Resolved 2026-10-20:** the user approved the proposed fix; implemented as `Quarantine::recover_interrupted_restores`, run at startup before retention (K-017). Rows pointing outside the quarantine root are not trusted. Leftovers of a finished cross-volume restore also come back as held, visible instead of leaking; nothing is deleted.

## Open items raised by the C1 tweak engine (2026-10-24)

- [x] 2026-10-24 (phase 6, C1): `docs/RESEARCH-FEATURES-2026-10.md` §C1 needs a registry-writing action, and the `Action` enum had none, so C1 had to come here before code. Proposed: a new mutating `Action::ApplyTweak` whose `Justification::Tweak` names the catalog entry and the prior value the user was shown; a `tweak_undo` table that records the real prior value (or "absent") **before** the write, plus the applied value; read-back after every write; undo only while the live value is still the one WinKnife wrote, so a later change by the user or another tool is never clobbered. HKCU only; HKLM stays out.
  - **Resolved 2026-10-24 (user, K-018):** "C1 ... oradan başla geliştirmeye devam et" — proceed as proposed, HKCU only, escalate anything that would need HKLM or an undo that cannot be proven. Brief: `docs/phases/PHASE-6-tweaks.md`.

## Open items raised by C4 — hosts editor (2026-10-24)

- [ ] 2026-10-24 (C4): the app stays unelevated; a save backs the current file up and stages the new bytes in app data, then relaunches WinKnife once through `runas` with `--write-hosts <staged> <blake3 of the file it backed up>`. The elevated helper only writes the fenced hosts path, refuses if the file changed since the backup, and exits — safest reading of "elevation via the existing runas helper pattern".
- [ ] 2026-10-24 (C4): a file whose lines end inconsistently (some CRLF, some LF) is saved with the dominant ending; consistent files round-trip byte-for-byte. A UTF-16 hosts file is shown read-only (Windows writes it ANSI/UTF-8); non-UTF-8 bytes are kept as Latin-1 so nothing is lost.
- [ ] 2026-10-24 (C4): backups are never removed by the app; the page only shows them in Explorer. If they pile up, a retention rule is a later decision.

## Open items raised by C2 — context-menu cleaner (2026-10-24)

- [ ] 2026-10-24 (C2): HKLM (machine-wide) entries are shown but never written: the row says "needs admin; not changed". Disabling them would need elevation through the hosts-style `runas` helper; not done without a decision.
- [ ] 2026-10-24 (C2): a per-user handler whose CLSID is also registered machine-wide is left alone, because blocking the CLSID would also hide the machine-wide entry. Left to the admin.
- [ ] 2026-10-24 (C2): `LegacyDisable` on HKCU verbs and the HKCU `Shell Extensions\Blocked` list are documented Explorer mechanisms, but whether the Windows 11 compact menu (and Explorer without a restart) honours them has not been checked by hand. Manual check wanted before the feature is announced.

## Open items raised by admin elevation (2026-10-25)

- [x] 2026-10-25 (K-021): "Always start as admin" is **not offered**. The only ways to start elevated without a UAC prompt each launch are a `requireAdministrator` manifest (every launch elevated, against least privilege, and a per-user unsigned install would show the UAC prompt on every start anyway) or a scheduled task with "run with highest privileges" (the hack the brief excludes; it also leaves a SYSTEM-visible persistence entry). A setting that only adds a UAC prompt to every start brings nothing over the "Restart as admin" button, so the button stays the only path.
- [ ] 2026-10-25: the startup page knows upfront which rows are HKLM / all-users (they fail without admin) but shows no hint before the action; that page belongs to the list-QoL work running in parallel, so the hint is left for a follow-up. Today such a row fails with the "needs administrator rights" note and the global banner.

## Open items raised by the tweaks catalogue (2026-10-25)

- [ ] 2026-10-25 (tweaks): the engine writes `REG_DWORD` only. Useful per-user settings stored as strings are left out: classic Windows 11 context menu (`CLSID\{86ca1aa0-...}\InprocServer32` default value), mouse acceleration (`MouseSpeed`, `MouseThreshold1/2`), menu show delay (`MenuShowDelay`) and Sticky Keys `Flags`. Adding `REG_SZ` support is an engine change (K-018), not done here.
- [ ] 2026-10-25 (tweaks): refused by the fence on purpose, so not offered: Bing results in Start search (`Search\BingSearchEnabled`), search box mode (`SearchboxTaskbarMode`) and the `Start_*` values (Start recommendations and similar). The fence blocks any path containing `search` or `start_`; relaxing it needs a decision.
- [ ] 2026-10-25 (tweaks): the Widgets button tweak (`TaskbarDa`) is protected on some recent Windows 11 builds; the write is rejected or reverted and the read-back check then reports it. Left in because it works on other builds; confirm on current 24H2/25H2.
- [ ] 2026-10-25 (tweaks): future "admin tweaks" item. Settings that need HKLM or elevation (telemetry level, Windows Update policies, consumer-features policy, Wi-Fi Sense, fast startup, disabling services) are not offered. They would need the elevation helper used for hosts and a per-tweak admin flag.
- [ ] 2026-10-25 (tweaks): "Restart Explorer" button not added. Restarting explorer.exe closes the user's open folder windows; the page shows a notice instead. Revisit if users ask.

## Open items raised by the EN/TR language work (2026-10-25)

- [ ] 2026-10-25 (K-022): text generated by the Rust backend is English only and shows as-is in the Turkish UI. That covers `Recommendation.reason` and notes from the brain, the cleaner, duplicates, empty-folders, startup, context-menu, tweaks, hosts, file-unlocker and restore-point modules (`ModuleManifest` names/descriptions, per-item reasons, per-item `ExecItemResult` errors), plus `AppError` messages in `commands.rs`, `core/executor.rs`, `core/quarantine.rs` and the platform wrappers. Not restructured here (scope: UI only). Proposed approach: the backend emits a stable key plus typed params next to the English text (`reasonKey: "cleaner.tempFiles"`, `reasonParams: {...}`); the UI looks the key up in `i18n.ts` and falls back to the English `reason` when the key is unknown. Rule data (`rules/*.json`) gets an optional `reasonKey` per rule; errors get an `ApiError.code` + params. English stays the source of truth, so a missing TR key degrades to English, never to an empty string. Do it module by module, brain/cleaner first (the most visible text).
- [ ] 2026-10-25 (K-022): the file-dialog filter name "WinKnife profile" (`api.ts`, `PROFILE_FILTERS`) and the language names in the selector ("English", "Türkçe", always native) are deliberately not translated.

## Open items raised by the repair kit (2026-10-25)

- [ ] 2026-10-25 (phase 7): `DISM /Online /Cleanup-Image /StartComponentCleanup` (and `/ResetBase`) not offered. It removes superseded components and makes installed updates impossible to uninstall, i.e. not reversible. Deferred; the AnalyzeComponentStore verdict only says whether Windows recommends it.
- [ ] 2026-10-25 (phase 7): verdicts are parsed from English output. On non-English Windows the parser falls back to exit codes (DISM: 0 = finished, 3010 = restart needed, 0x800F081F = repair source missing, other non-zero = failed); sfc's exit codes are undocumented, so a non-English sfc run reports "finished, see log". Localised patterns could be added per language later.
- [ ] 2026-10-25 (phase 7): Cancel kills only the `dism.exe`/`sfc.exe` child; a `DismHost.exe` or TrustedInstaller worker it started may keep running until it finishes on its own. Safest interpretation: no process-tree kill of system services.
- [ ] 2026-10-25 (phase 7): repair logs (`repair-*.log` in the log folder) are not pruned. They are small; add retention if they pile up.

## Open items raised by the network module (2026-10-25)

- [ ] 2026-10-25 (K-023): no "end this process" action on the Network page. Killing a process by PID is a destructive, hard-to-undo action that bypasses the analyze-first promise; it needs its own decision (confirm, protected-process fence, window of reversibility) before it is built. The page shows PID and exe path so the user can act in Task Manager.
- [ ] 2026-10-25 (K-023): winsock reset and IP stack reset are not offered. Both need admin, both require a reboot and neither can be undone from WinKnife; K-021 keeps WinKnife unelevated.
- [ ] 2026-10-25 (K-023): no firewall changes (block a port or program). Would be a persistent system change needing admin and a rollback story; revisit with the admin-tweaks item.
- [ ] 2026-10-25 (K-023): no network calls of any kind, so remote addresses are shown as raw IPs (no reverse DNS, no geo-IP, no reputation lookup). Revisit only as an explicit opt-in.
- [ ] 2026-10-25 (K-023): the list is a snapshot on open and on Refresh, not a live monitor; bytes per connection need ETW or admin and are out of scope.

## Open items raised by the environment-variable editor (2026-10-25, phase 7)

- [ ] 2026-10-25 (phase 7): deleting a user variable is a registry change, not a file deletion, so it goes through C1 like any other write. `tweak_undo.applied_*` is NOT NULL, so a delete stores `REG_NONE` + empty bytes as "absent after" instead of adding a migration — safest interpretation taken: undo of a delete only runs while the value is still absent, then restores the saved value and kind.
- [ ] 2026-10-25 (phase 7): undo is one step, like C1 (a newer change supersedes the older row). Multi-step history was not asked for — left out.
- [ ] 2026-10-25 (phase 7): env changes bypass the executor/`TweakRouter` (no scan, no recommendations); the page calls `env_vars_apply` / `env_vars_undo` directly, like the hosts editor. Revisit if C7 drift check should cover `env:` rows.
- [ ] 2026-10-25 (phase 7, C5): `%ProgramData%` children are always info-only, because ProgramData is tree-protected and no pack rule lifts it. Leftovers there are listed but cannot be quarantined — safest reading; revisit only with a per-vendor allow rule.
- [ ] 2026-10-25 (phase 7, C5): developer/tool cache folders (no Uninstall entry, e.g. `.npm`-style vendor folders) can look like leftovers. Mitigated by the Needs Review cap and per-item selection; no allow-list was built.
- [ ] 2026-10-25 (phase 7, C5): name matching is deliberately generous towards "installed" (normalised names, publisher/display/install-location leaves, words of 4+ chars); folder names under 3 chars are skipped. A missed leftover is the accepted failure mode.
- [ ] 2026-10-25 (phase 7, C5): `start_uninstaller_scan` takes optional `roots` (used by the wiring probe); explicit roots still go through the brain, the protected-path gates and the executor re-check.
- [ ] 2026-10-25 (phase 7, C5): the brief named the decision K-022, which was already taken; it is recorded as K-026. Renumber if a parallel PR lands K-026 first.
