<!-- durum: aktif -->
# Feature research: what to build next (2026-10)

Research input for [ROADMAP](ROADMAP.md) and [BACKLOG](modules/BACKLOG.md). It changes neither; promoting a candidate is a separate decision. Collected 2026-10-24.

## 1. Method and limits

WinKnife has about 1 star and 45 downloads, so there is no user feedback of its own. The evidence comes from what people ask of, and complain about in, comparable tools:

- **Tweak/debloat tools:** [Chris Titus WinUtil](https://github.com/ChrisTitusTech/winutil), [Win11Debloat](https://github.com/Raphire/Win11Debloat), [Sophia Script](https://github.com/farag2/Sophia-Script-for-Windows), [O&O ShutUp10++](https://www.oo-software.com/en/shutup10).
- **Cleaners and uninstallers:** [BleachBit](https://github.com/bleachbit/bleachbit), CCleaner, [Wise Disk Cleaner](https://www.wisecleaner.com/wise-disk-cleaner.html), [Glary Utilities](https://www.glarysoft.com/glary-utilities/), [Revo](https://www.revouninstaller.com/revo-uninstaller-free-download/), [BCUninstaller](https://github.com/Klocman/Bulk-Crap-Uninstaller), [Microsoft PC Manager](https://pcmanager.microsoft.com/), [Storage Sense](https://support.microsoft.com/en-us/windows/manage-drive-space-with-storage-sense-654f6ada-7bfc-45e5-966b-e24aded96ad5).
- **Duplicates and disk maps:** [Czkawka](https://github.com/qarmin/czkawka), [dupeGuru](https://github.com/arsenetar/dupeguru), [WinDirStat](https://github.com/windirstat/windirstat), [WizTree](https://diskanalyzer.com/), [SpaceSniffer](https://www.uderzo.it/main_products/space_sniffer/index.html).
- **Utility suites:** [PowerToys](https://github.com/microsoft/PowerToys), [Everything](https://www.voidtools.com/), [Patch My PC](https://patchmypc.com/home-updater), [UniGetUI](https://github.com/Devolutions/UniGetUI).
- **Signals read:** GitHub issues (comments, thumbs-up reactions), Hacker News threads, AlternativeTo comments, vendor feature pages.

Three research agents collected this on 2026-10-24; raw notes were not committed. Limits that matter:

- **Demand is qualitative.** The most-reacted WinUtil issue has 18 reactions. Numbers below are signals, not counts of users.
- **Not readable:** Reddit directly (blocked; a few scores come from the PullPush archive and are archived values), YouTube, and several vendor pages (CCleaner, Wise Care, AllDup, some WizTree pages returned 403/404). Nothing is inferred from them.
- **Bias:** Hacker News skews technical; self-promotion threads (FREGONATOR, Kudu, the jv16 author) are flagged where used; several peers' sources are search-excerpt only and are marked "unverified".
- **Already-planned items** are marked **planned**: they are in BACKLOG or ROADMAP, so this document supplies evidence and a design angle, not a new idea. **NEW** means not found in BACKLOG, ROADMAP or the module specs.

Size: **S** = one screen or rule change on existing infrastructure; **M** = new module with its own tests; **L** = needs new infrastructure (new `Action`, new persistence) or several screens.

## 2. Reading the field

1. **Undo is the gap in every tweak tool, and it is where users get hurt.** WinUtil's own docs say it ["doesn't track original values"](https://winutil.christitus.com/guides/tweaks/). Users are left with "managed by your organization" after policy tweaks ([#4491](https://github.com/ChrisTitusTech/winutil/issues/4491), [#4242](https://github.com/ChrisTitusTech/winutil/issues/4242), [#1918](https://github.com/ChrisTitusTech/winutil/issues/1918), [#81](https://github.com/ChrisTitusTech/winutil/issues/81)); a maintainer says ["ideally all tweaks should be reversible"](https://github.com/ChrisTitusTech/winutil/issues/410); Undo itself broke things ([#3881](https://github.com/ChrisTitusTech/winutil/issues/3881), [#4039](https://github.com/ChrisTitusTech/winutil/issues/4039)). Win11Debloat's restore point corrupted Sticky Notes data and Undo broke ([#576](https://github.com/Raphire/Win11Debloat/issues/576), "Lost my restore points"). O&O's free version documents no undo.
2. **Tweaks break Windows Update, search and the Start menu.** WinUtil [#3876](https://github.com/ChrisTitusTech/winutil/issues/3876) (58 comments), [#3214](https://github.com/ChrisTitusTech/winutil/issues/3214), [#3755](https://github.com/ChrisTitusTech/winutil/issues/3755), [#4446](https://github.com/ChrisTitusTech/winutil/issues/4446), [#5130](https://github.com/ChrisTitusTech/winutil/issues/5130); Win11Debloat [#361](https://github.com/Raphire/Win11Debloat/issues/361) (39 comments, Start search broke), [HN 36969850](https://news.ycombinator.com/item?id=36969850).
3. **Settings drift after Windows updates.** O&O's paid Premium monitors and re-applies ([HN 49564818](https://news.ycombinator.com/item?id=49564818) reports resets); Win11Debloat users ask the same ([HN 45810960](https://news.ycombinator.com/item?id=45810960), [45811019](https://news.ycombinator.com/item?id=45811019)). WinKnife could offer the check free.
4. **Trust is the product in cleaners.** The CCleaner v7 "auto-installed without UAC" thread has score 1141 and 440 comments ([archive](https://api.pullpush.io/reddit/search/comment/?link_id=1ptyhlg&sort_type=score&sort=desc&size=10)); Floxif trojan in v5.33 ([Wikipedia](https://en.wikipedia.org/wiki/CCleaner)). Unknown authors are distrusted too ([FREGONATOR thread](https://reddit.com/r/pcmasterrace/comments/1qtv91e/i_built_an_opensource_alternative_to_ccleaner/): "random person on Reddit"). Antivirus false positives hit free tools: BCU [#770](https://github.com/Klocman/Bulk-Crap-Uninstaller/issues/770), WinUtil [#1742](https://github.com/ChrisTitusTech/winutil/issues/1742), Win11Debloat [#663](https://github.com/Raphire/Win11Debloat/issues/663); fake download sites worry users ([Czkawka #1583](https://github.com/qarmin/czkawka/issues/1583)). `irm | iex` is distrusted ([HN 45312264](https://news.ycombinator.com/item?id=45312264)).
5. **UI and accessibility beat features.** BCU dark mode [#228](https://github.com/Klocman/Bulk-Crap-Uninstaller/issues/228) (56 reactions, 55 comments) is the largest single ask found among cleaners. HiDPI/font size: Czkawka [#787](https://github.com/qarmin/czkawka/issues/787) (20), [#863](https://github.com/qarmin/czkawka/issues/863) (19). Screen readers: BleachBit [#631](https://github.com/bleachbit/bleachbit/issues/631), [#719](https://github.com/bleachbit/bleachbit/issues/719).
6. **Duplicate folders, and "which copy do I keep".** Czkawka's top open feature asks: [#676](https://github.com/qarmin/czkawka/issues/676) (19), [#1182](https://github.com/qarmin/czkawka/issues/1182) (16), [#976](https://github.com/qarmin/czkawka/issues/976) (9). dupeGuru users can't choose which duplicates go ([AlternativeTo](https://alternativeto.net/software/dupeguru/about/)); PC Manager's large/downloaded lists have no quick delete ([LazyAdmin review](https://lazyadmin.nl/win-11/microsoft-pc-manager/)).
7. **Overlap with PowerToys is real.** PowerToys already ships Hosts File Editor, Environment Variables, Awake and File Locksmith ([repo](https://github.com/microsoft/PowerToys)). WinKnife's `keep_awake`, `file_unlocker` and the planned env-var/hosts editors must win on undo and plain-language reasons, not on existing.
8. **Price is not the axis.** BleachBit, BCU, Czkawka, dupeGuru and PC Manager are fully free; Revo's page lists the same features for Free and Pro ([page](https://www.revouninstaller.com/revo-uninstaller-free-download/)). The only clear paywalls found: O&O Premium re-apply, WizTree commercial use.

## 3. Comparison matrix

`yes` = documented in a source above; `no` = documented absent; `—` = out of the tool's scope; `?` = not verified.

| Tool | Junk clean with preview | Reversible delete | System tweaks | Tweak undo | Leftover scan | Duplicates | Disk map | Open source |
|---|---|---|---|---|---|---|---|---|
| **WinKnife 0.3.0** | yes | yes (quarantine, 7 days) | no (startup entries only) | — (planned) | no (planned) | files only | yes | yes (MIT) |
| WinUtil | — | — | yes | partial ("doesn't track original values") | — | — | no ([#42](https://github.com/ChrisTitusTech/winutil/issues/42) asks for WizTree) | yes |
| O&O ShutUp10++ | — | — | yes | no in free version | — | — | — | no |
| Win11Debloat | — | — | yes | restore point / Undo, reported broken ([#576](https://github.com/Raphire/Win11Debloat/issues/576)) | — | — | — | yes |
| Sophia Script | — | — | yes | per-tweak restore promised; script, no preview | — | — | — | yes |
| PowerToys | — | — | utilities (hosts, env vars) | ? | — | — | — | yes |
| BleachBit | yes | no (shred) | — | — | — | — | — | yes (GPL) |
| CCleaner | ? | ? | ? | ? | ? | ? | ? | ? |
| Wise Disk Cleaner | ? | no (secure delete) | — | — | uninstall residue | — | — | ? |
| BCUninstaller | — | — | — | — | yes (accuracy criticised) | — | — | yes (Apache-2.0) |
| Revo | — | multi-level backup (vendor page) | — | — | yes | — | — | ? |
| Czkawka | yes (temp, empty) | ? | — | — | — | files; folders requested | — | yes |
| dupeGuru | — | ? | — | — | — | files | — | yes (GPL) |
| WinDirStat | — | no (review in Explorer, then delete) | — | — | — | — | yes | yes (GPL) |
| WizTree | — | no safe delete found | — | — | — | — | yes | no |

Sources for each row are in section 2 or in the candidate entries. Where WinKnife's cell says "no", it is from `docs/` and the module list, not a code audit.

## 4. Candidates, ranked

Ranking weighs demand signal, differentiation against what peers get wrong, fit with the rules, and size. Rules checked for every item: scans read-only; deletes go to quarantine, other changes to an undo record; no network without opt-in; protected paths via `brain::protected_paths()`; every row carries a `reason`.

### C1. Reversible tweak engine (snapshot, verify, undo) — planned ("reversible tweaks", ROADMAP later)

- **Need:** the strongest finding across all tweak peers (section 2, items 1-2). Users want fewer clicks to a tidier Windows, but the field has no tool that restores what it changed.
- **Peers' flaw:** WinUtil records no original values; Win11Debloat's restore point and Undo failed ([#576](https://github.com/Raphire/Win11Debloat/issues/576)); Sophia ships as a script without preview ([#141](https://github.com/farag2/Sophia-Script-for-Windows/issues/141): a second run removed all UWP apps); O&O free has no undo.
- **Design angle (NEW):** each tweak stores the real prior value (or "absent") in SQLite before writing; shows the exact registry path, old and new value and a source link before applying; read-back verifies after; says honestly when an effect cannot be undone (state per tweak: `reversible` / `reversible after restart` / `not undoable`), and ships none of the last kind. Shows "restart needed" ([#2644](https://github.com/ChrisTitusTech/winutil/issues/2644)) and plain-language names instead of jargon ([#2632](https://github.com/ChrisTitusTech/winutil/issues/2632)). Hard fence like K-005: the catalog cannot contain Windows Update, search indexing, Start menu or Defender-disabling entries (see [#3876](https://github.com/ChrisTitusTech/winutil/issues/3876), [#361](https://github.com/Raphire/Win11Debloat/issues/361)).
- **Rule fit:** the `Action` enum has no registry action. Needs a new variant (e.g. `ApplyTweak`) and an undo table, so this goes to DECISIONS-NEEDED before code. The env-var-editor seed already assumes "registry writes behind the same reversible-executor discipline".
- **Size:** L. **Module:** `tweaks` (absorbs `debloat-lite`, `services-lite` seed ideas that stay inside the fence).

### C2. Context-menu cleaner — planned (BACKLOG Tier A)

- **Need:** [PowerToys #33](https://github.com/microsoft/PowerToys/issues/33), context-menu customisation: 357 reactions, 50 comments, open since 2019-05-08. That issue is about customising the menu, not cleaning stale shell extensions, so it shows demand for control over the menu rather than for exactly this module.
- **Peers' flaw:** not found in any cleaner reviewed except as paid/Wise-style startup and context-menu management claims (unverified).
- **Rule fit:** the seed already uses rename-prefix, which is reversible; scan is a read of shell keys; reason = what the entry is and which app owns it; unknown owners stay "needs review". Uses C1's undo store if present, otherwise its own rename record.
- **Size:** M. **Module:** `context_menu` (seed name `context-menu-cleaner`).

### C3. Duplicate folders and a "keep which copy" chooser — NEW

- **Need:** section 2, item 6. Czkawka's top open asks plus [#1324](https://github.com/qarmin/czkawka/issues/1324) (select all duplicates in one folder, 9), [#1431](https://github.com/qarmin/czkawka/issues/1431) (ignore matches, 10), [#1295](https://github.com/qarmin/czkawka/issues/1295) (load a saved list, 11); dupeGuru [#296](https://github.com/arsenetar/dupeguru/issues/296) (folder-aware rules, 56 comments, unresolved design debate).
- **Peers' flaw:** every peer deletes. WinKnife's quarantine makes aggressive auto-selection safe, which is the differentiator.
- **Design angle:** group folders whose file sets are equal (hash of sorted child hashes); per-group keep-rule (oldest, newest, in preferred folder); everything unticked goes to quarantine; an "ignore this group" list persisted in SQLite.
- **Rule fit:** reuses the existing duplicates hasher and `Quarantine` action; protected-path gate unchanged.
- **Size:** M. **Module:** extend `duplicates`.

### C4. Hosts editor — planned (BACKLOG Tier B)

- **Need:** WinUtil users lost their hosts file: 0-byte hosts ([#2277](https://github.com/ChrisTitusTech/winutil/issues/2277)), also [#4039](https://github.com/ChrisTitusTech/winutil/issues/4039), [#3657](https://github.com/ChrisTitusTech/winutil/issues/3657). PowerToys already ships an editor, so undo is the only edge.
- **Rule fit:** timestamped backup before every write, write by temp file then replace, parse and validate before saving, diff preview, one-click restore. Seed spec already says this.
- **Size:** S to M. **Module:** `hosts`.

### C5. Uninstaller+ with leftover scan that does not uninstall — planned (BACKLOG Tier B)

- **Need:** BCU users call leftover cleanup ["very inaccurate"](https://alternativeto.net/software/bulk-crap-uninstaller/about/) and slow to start; [#736](https://github.com/Klocman/Bulk-Crap-Uninstaller/issues/736) asks for scan without uninstalling (3 reactions, 0 comments). Low reaction count, but BCU and Revo are the free benchmarks and WinKnife has the cleaner engine to reuse.
- **Rule fit:** leftovers are files and folders only, scored by the brain with vendor-path rules, deleted via quarantine. No registry-cleaner behaviour (non-goal; BleachBit [#1581](https://github.com/bleachbit/bleachbit/issues/1581) has 3 reactions and CCleaner threads call registry cleaning pointless).
- **Size:** L. **Module:** `uninstaller`.

### C6. Repair kit with live output and a plain verdict — planned (ROADMAP v0.3.0)

- **Need:** WinUtil asks for visible scan output ([#3442](https://github.com/ChrisTitusTech/winutil/issues/3442), discussion [#3413](https://github.com/ChrisTitusTech/winutil/discussions/3413)).
- **Design angle:** `sfc`/`DISM` run only on click, each button shows the exact command, live output streamed, a one-line verdict ("no integrity violations" / "repaired" / "needs restart") and a saved log. Read-oriented, so rule fit is easy; mark it as an exception wherever a command changes the system.
- **Optional add-on (NEW):** component-store analysis (read-only `DISM /AnalyzeComponentStore`) answers [WinUtil #3337](https://github.com/ChrisTitusTech/winutil/issues/3337) (WinSxS slimming) without deleting anything; cleanup stays behind the same consent step.
- **Size:** M. **Module:** `repair`.

### C7. Drift check for applied tweaks — NEW (depends on C1)

- **Need:** section 2, item 3. O&O gates re-apply behind a paid tier.
- **Design angle:** compares current values with what C1 recorded as applied; lists changed ones with the reason (for example "changed since 2026-11-02 after a Windows update"); re-apply is a button, never automatic, and is itself undoable. Also shows read-only "is something blocking Windows Update" status for tweaks WinKnife did not make, using the same fence as C1.
- **Rule fit:** pure read plus consented write. **Size:** M. **Module:** inside `tweaks`.

### C8. Display scaling and accessibility pass — NEW

- **Need:** section 2, item 5. Dark mode already exists (themes store in the codebase), which was BCU's biggest ask.
- **Scope:** UI scale setting (100-200%, Wise offers this: [page](https://www.wisecleaner.com/wise-disk-cleaner.html)), keyboard-only operation of every list and dialog, screen-reader labels on the findings list, contrast check in both themes. Fits the project's calm-UI line and [UniGetUI's top issues](https://github.com/Devolutions/UniGetUI/issues/5012), which are UI taste.
- **Size:** S to M. **Module:** none (shell and components).

### C9. Provenance and verification page — NEW (release pipeline, not a module)

- **Need:** section 2, item 4. K-014 settles signing (unsigned for now); this adds checks users can run themselves.
- **Scope (inference from the evidence, not a peer feature):** a "Verify this download" section in README and the in-app About with SHA-256 and the official URLs, GitHub build attestations for release assets (free for public repos), an in-app list of what the app never does (no network, no auto-update, no background service), and a pre-written antivirus false-positive note. Tagging releases stays the user's decision (K-015).
- **Size:** S. **Module:** none.

### C10. Settings profile export/import — planned (ROADMAP later)

- **Need:** WinUtil [#3138](https://github.com/ChrisTitusTech/winutil/issues/3138), [#3722](https://github.com/ChrisTitusTech/winutil/issues/3722), [#3566](https://github.com/ChrisTitusTech/winutil/issues/3566), discussion [#448](https://github.com/ChrisTitusTech/winutil/discussions/448); BleachBit [#1387](https://github.com/bleachbit/bleachbit/issues/1387); Winhance keeps removed apps removed and saves a config ([HN 47711544](https://news.ycombinator.com/item?id=47711544)).
- **Rule fit:** a JSON file of chosen tweak ids and settings; import shows the diff and applies through C1. **Size:** S after C1. **Module:** `tweaks` and Settings.

### Next tier (worth doing, lower or unclear signal)

| # | Candidate | Status | Signal and note | Size |
|---|---|---|---|---|
| C11 | Scheduled scan that stages a report; optional auto-quarantine of the green bucket | planned (Tier C) | Wise has AutoClean free ([page](https://www.wisecleaner.com/wise-disk-cleaner.html)). Reversible redesign: scan runs on a schedule, cleaning only goes to quarantine, shows a notification with the numbers; no silent cleanup of Yellow items. K-016 satisfied. | M |
| C12 | Clean contents but keep the folder (rule option) | NEW | BleachBit [#2073](https://github.com/bleachbit/bleachbit/issues/2073) (2 reactions). Check `docs/RULES-FORMAT.md` first; not verified whether a rule can express it. | S |
| C13 | Treemap tag-and-quarantine flow (SpaceSniffer-style tags) | NEW | SpaceSniffer has tags and filters ([site](https://www.uderzo.it/main_products/space_sniffer/index.html)); WinDirStat sends users to Explorer ([README](https://github.com/windirstat/windirstat)); PC Manager has no quick delete. Verify what the treemap already offers before scoping. | M |
| C14 | Network screen: ports, per-process usage, DNS flush | planned (ROADMAP v0.3.0) | [PowerToys is not a competitor here](https://github.com/microsoft/PowerToys); BleachBit asked for DNS flush ([#1698](https://github.com/bleachbit/bleachbit/issues/1698)). | M |
| C15 | Env-var editor | planned (Tier B, ROADMAP v0.3.0) | PowerToys has one; differentiate with snapshot and diff preview. | S |
| C16 | Big-file archiver | planned (Tier A) | No specific peer signal found; PC Manager's large-file view lacks quick actions. | M |
| C17 | Turkish UI and language picker | open question already | WinUtil language asks: [#3128](https://github.com/ChrisTitusTech/winutil/issues/3128), [#2993](https://github.com/ChrisTitusTech/winutil/issues/2993). Currently 117/352 keys. | M |
| C18 | Similar-image duplicates (HEIC, JPEG XL) | NEW, risk of scope creep | dupeGuru [#455](https://github.com/arsenetar/dupeguru/issues/455) (17), [#1109](https://github.com/arsenetar/dupeguru/issues/1109) (7). Only after C3. | L |
| C19 | Debloat picker with a list of installed apps | see "Not compatible" | WinUtil [#4695](https://github.com/ChrisTitusTech/winutil/issues/4695), [HN 36970471](https://news.ycombinator.com/item?id=36970471). A read-only "what is installed and who ships it" view, linking to Settings, is compatible. | S |

## 5. Top 10: build next

Order is dependency-aware: C1 is the foundation for C7 and C10.

1. **C1 Reversible tweak engine** (L). The single largest gap in the field and the on-brand answer to it; needs a decision on a new `Action` first.
2. **C2 Context-menu cleaner** (M). Biggest single reaction count found (357); reversible by design already in BACKLOG.
3. **C3 Duplicate folders plus keep-which-copy chooser** (M). Czkawka's top open requests; quarantine lets the selection be aggressive.
4. **C4 Hosts editor with backup and diff** (S-M). Competitors have destroyed users' hosts files; PowerToys has no undo to compete with.
5. **C5 Uninstaller+ with separate leftover scan** (L). Fills the gap between free benchmarks BCU and Revo using the existing cleaner engine.
6. **C6 Repair kit with live output and verdict** (M). Cheap, read-oriented, and asked for by WinUtil users.
7. **C7 Drift check** (M, after C1). Free version of O&O's paid feature, consent-gated.
8. **C8 Display scaling and accessibility pass** (S-M). Cleaners' loudest complaints are UI; cheap trust win.
9. **C9 Provenance and verification page** (S). Trust is the product; costs nothing and does not depend on code signing.
10. **C10 Settings profile export/import** (S, after C1). Repeated across WinUtil, BleachBit, Winhance.

## 6. Not compatible with the rules, or out of lane

| Idea | Seen in | Why not | Reversible redesign |
|---|---|---|---|
| Disable Windows Update | WinUtil tweaks ([#3876](https://github.com/ChrisTitusTech/winutil/issues/3876) shows the damage) | Already ROADMAP "not planned"; breaks security patching | Read-only status of what blocks updates (C7) |
| Remove Edge, OneDrive or Appx packages | WinUtil [#727](https://github.com/ChrisTitusTech/winutil/issues/727), [#1075](https://github.com/ChrisTitusTech/winutil/issues/1075), [#3080](https://github.com/ChrisTitusTech/winutil/issues/3080) (removal fails) | ROADMAP "not planned"; app data and servicing state cannot be restored by an undo record | Listing only (C19) |
| MicroWin, service-disabling packs | WinUtil | ROADMAP "not planned"; breaks features in ways users report as search, Start, Update failures | None |
| Registry cleaner | CCleaner, BleachBit [#1581](https://github.com/bleachbit/bleachbit/issues/1581) | BACKLOG explicit non-goal; users call it pointless ([thread comments](https://api.pullpush.io/reddit/search/comment/?link_id=1ptyhlg&sort_type=score&sort=desc&size=10)) | None |
| Secure delete / shredder, free-space wipe | BleachBit, Wise | Permanent by design; conflicts with the hard rule and K-016 | None; quarantine purge stays the only delete |
| Direct delete without quarantine | user note (K-016) | K-016 | Faster quarantine instead |
| Driver updater, RAM booster, antivirus features | BACKLOG non-goals | Risk or placebo | None |
| App installer or updater | WinUtil installs ([#1782](https://github.com/ChrisTitusTech/winutil/issues/1782), 185 comments), [Patch My PC](https://patchmypc.com/home-updater), [UniGetUI](https://github.com/Devolutions/UniGetUI) | ROADMAP: winget install/upgrade not before v1.0; others cover it | None |
| Name search like Everything | [HN 40451491](https://news.ycombinator.com/item?id=40451491) | Not a gap: search-only tool people already like | None |
| Defender scan button | WinUtil [#4943](https://github.com/ChrisTitusTech/winutil/issues/4943) | One request; not worth a screen | None |
| OLED text, virtual desktops, text expander, hot corners | PowerToys [#25595](https://github.com/microsoft/PowerToys/issues/25595) (1086), [#58](https://github.com/microsoft/PowerToys/issues/58) (504), [#5074](https://github.com/microsoft/PowerToys/issues/5074) (221), [#1305](https://github.com/microsoft/PowerToys/issues/1305) (194) | Large reaction counts but outside the cleanup/safety lane | None |

## 7. Gaps in this research

- No evidence from real WinKnife users; the first soak of v0.3.0 could change the order.
- CCleaner and Glary paywall details, AllDup features, Disk Cleanup options and Storage Sense sentiment were not verified.
- Whether C3, C12 and C13 already exist in the code is judged from `docs/`, not a code audit; check before scoping.
- Treemap speed against WizTree's MFT-based scan was not measured.

## 8. Licences

No code, text or data was copied from any source; the links are attribution for ideas and evidence only, so `docs/LISANSLAR.md` needs no new row.
