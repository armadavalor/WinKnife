<!-- durum: aktif -->
# Launch posts (v0.3.0)

Drafts for the public launch. **Nothing here is posted until the maintainer
says so**, and the Show HN post waits for the cold-test gate in
[COLD-TEST.md](COLD-TEST.md#launch-gate).

Rewrite each draft in your own words before posting. Hacker News asks for
comments written by people, and on Reddit a templated post reads as spam.
These drafts list the facts and the order to give them in.

## Facts checked against v0.3.0 (2026-10-19)

Use only these. Recheck them if a newer release ships before posting.

| Claim | Source |
| --- | --- |
| v0.3.0 released 2026-10-19; installer 4.2 MB, portable ZIP 6.0 MB, `SHA256SUMS.txt` | Release page |
| Ten tools: Disk Cleaner, Duplicate Finder, Disk map, Startup, Shutdown Timer, Empty Folders, File Unlocker, Keep Awake, Restore Point, Legacy Panels (22 classic tools) | README status table |
| Rust backend, Tauri v2, Svelte 5 UI; MIT | README, LICENSE |
| Scans are read-only; every suggestion carries a reason; cleanup needs confirmation | README "Safety and privacy" |
| Cleaned files go to quarantine and can be restored **while retained**; default retention 7 days, age and size limits purge, including at startup | README; `core/quarantine.rs` `DEFAULT_RETENTION_DAYS` |
| Emptying the recycle bin is permanent, starts unticked and has its own confirmation | README, K-007 |
| Protected system locations can never receive a cleanup action | README, K-004 |
| No data is sent over the network; the LLM layer is not implemented | README "Privacy" |
| Not a registry cleaner, driver updater, antivirus or FPS booster | README |
| **Unsigned**: SmartScreen warns; VirusTotal showed 3 of ~70 generic ML detections on 0.2.0 | README "Code signing policy", DEVIR |

Do not claim: winget availability (the
[winget-pkgs PR](https://github.com/microsoft/winget-pkgs/pull/452789) is
still open), user numbers, speed benchmarks, "everything is reversible",
"safe on any PC", or that any outside party reviewed the code.

Fill `<cold-test summary>` from `cold-test-scores.csv`, for example "five
people who had never seen it tried it cold; four installed it unaided, all
who reached quarantine restored their file". Use the real numbers, including
the bad ones.

## Schedule

| When (UTC) | Where | Condition | Goal |
| --- | --- | --- | --- |
| Fri 2026-10-23 16:00 | Show HN | Cold-test gate passed; maintainer free for the next 4 hours to answer | 5 substantive comments, 2 people reporting they tried it |
| Wed 2026-10-28 15:00 | r/software | Wednesday self-promotion slot, correct flair, account karma enough to post | 5 actionable replies, 2 trial reports |
| Thu 2026-10-29 16:00 | r/Windows11 | **Only after modmail permission**; use the App flair | 3 Windows-specific trial reports |
| Thu 2026-11-05 | — | Review | 250 unique visitors, +50 setup downloads, 10 first scans reported by people outside the cold test |

Record GitHub traffic (views, unique visitors, referrers, release download
counts) on 10-20, 10-27 and 11-05. GitHub keeps only 14 days of traffic, so
save daily rows, don't add up overlapping totals.

If the gate slips, move everything back by the same amount. Keep r/software
on a Wednesday.

## Show HN

**Title** (80 character limit):

> Show HN: WinKnife – Windows cleanup that asks first and can restore

**URL:** `https://github.com/armadavalor/WinKnife`

**First comment** (post it yourself right after submitting):

> I built WinKnife because every Windows cleaner I tried either deleted
> things without showing me what or bundled itself with "PC boosting". It's
> a small open-source toolkit (Rust + Tauri v2, Svelte UI, MIT, 4.2 MB
> installer) with ten tools: disk cleaner, duplicate finder, disk map,
> startup manager, empty folders, file unlocker, restore points, and a few
> smaller ones.
>
> The cleanup rule is: scan read-only, show a reason for every suggestion,
> change nothing without approval, and move files to a quarantine instead
> of deleting them. You can restore from quarantine while it's retained
> (7 days by default; age and size limits purge it). The one permanent
> action is emptying the recycle bin, which starts unticked and has its
> own confirmation. System folders like C:\Windows and Program Files can't
> get a cleanup action at all; that's enforced in the Rust code, not in the
> rule files. Nothing is sent over the network.
>
> <cold-test summary>
>
> Caveats: releases are unsigned (I applied for free OSS signing and was
> turned down for lack of visibility), so SmartScreen will warn. SHA-256
> sums are on the release page. Please try it in a VM or Windows Sandbox
> first. It's also not a registry cleaner or a "speed up your PC" tool.
>
> What I'd most like to hear: where you'd hesitate before clicking Clean,
> and whether you could find the restore without reading docs.

Be online for the first hours. Answer criticism of the unsigned binary and
of "yet another cleaner" with facts from the table above. Don't argue.

## r/software

Post on a **Wednesday** with the self-promotion flair. Check the sidebar
rules again on the day: they change.

**Title:**

> WinKnife: free, open-source Windows cleanup that shows why, asks first and can restore

**Body:**

> I'm the developer. WinKnife is a small open-source Windows toolkit
> (MIT, Rust/Tauri, 4.2 MB installer, no ads, no telemetry, no account).
>
> What makes it different from most cleaners:
>
> - scans are read-only; every suggestion says why it's there and how risky it is
> - nothing changes until you approve it
> - cleaned files go to a quarantine you can restore from (kept 7 days by default)
> - system folders can never be offered for cleanup
> - emptying the recycle bin is the only permanent action, and it has its own warning
>
> Besides the cleaner there's a duplicate finder, disk map, startup manager,
> empty-folder finder, file unlocker, restore points, keep-awake, shutdown
> timer and a page that opens 22 classic Windows tools.
>
> <cold-test summary>
>
> Honest caveats: the installer is unsigned, so SmartScreen will warn
> (SHA-256 sums are on the release page). It isn't a registry cleaner or
> FPS booster. Try it in Windows Sandbox or a VM first if you like.
>
> A quick way to judge it: scan without cleaning, look at what it suggests
> and why, clean something harmless, then restore it.
>
> https://github.com/armadavalor/WinKnife
>
> What would you need to see before trusting a cleaner on your own PC?

## r/Windows11 (on hold until modmail permission)

Send modmail first: one line on what the app is, that it's free and
open source, and ask whether a single post with the App flair is allowed.
Post only after a written yes.

**Title:**

> WinKnife: review cleanup suggestions and restore quarantined files on Windows 11

**Body:**

> I maintain WinKnife, a free open-source Windows utility (MIT). Its
> cleaner scans read-only, explains each suggestion, asks before changing
> anything and moves files to a quarantine you can restore from.
>
> I'm after Windows 11 feedback on three things: the unsigned-download
> warning, the first scan, and finding the restore after restarting the
> app.
>
> The installer is unsigned; SHA-256 sums are on the release page. Windows
> Sandbox is a good place to try it.
>
> https://github.com/armadavalor/WinKnife
>
> (Moderators approved this post.) <add if true: This post was drafted
> with AI assistance and edited by me.>

## After posting

- Reply to every question in the first day.
- Each bug report gets a GitHub issue, linked back in the thread.
- Note on 11-05, separately from the cold test: first scans reported,
  quarantine → restore reported, people returning after 7+ days, and any
  outside issue, PR or write-up. That evidence goes into the Codex for OSS
  application.
