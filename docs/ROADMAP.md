<!-- durum: aktif -->
# Roadmap

WinKnife grows one tool at a time. A tool is admitted when it solves a real,
recurring Windows annoyance, fits on one screen or the scan → review → act flow,
and every change it makes can be undone. Dates are not promised; milestones on
GitHub track progress.

Sources: the three-model brainstorm of 2026-10-03 (backed up outside the repo),
[modules/BACKLOG.md](modules/BACKLOG.md), and a review of
[Chris Titus Tech's WinUtil](https://github.com/ChrisTitusTech/winutil) (MIT) for
ideas that fit our "analyze first, reversible" promise.

## v0.1.0 — first release

The five current tools, as they are. No LLM layer.

Blocking before the tag:
- Fresh Windows VM install / restore / uninstall acceptance (docs/RELEASING.md).
- CI green on the release commit, including the symlink safety tests (they need
  Developer Mode or elevation and cannot be skipped on CI).
- Check that two app launches cannot arm the persisted shutdown timer twice. If
  they can, single-instance moves into v0.1.0.

## v0.2.0 — "everyday companion"

| Item | Notes |
| --- | --- |
| Single instance | Second launch focuses the existing window. |
| `--module <id>` launcher | Open WinKnife straight on one tool; used by shortcuts and the tray. |
| Module enable flags | Hide tools you do not use. |
| Sticky notes | Local notes with their own trash (soft delete, restorable). Optional, opt-in autostart via HKCU `Run`. Supersedes the 2026-08-06 rejection in BACKLOG. |
| Empty-folder hunter | Built on the shared scanner; quarantine-safe. Good first issue. |
| Keep-awake | Temporary "do not sleep" with a visible countdown; uses `SetThreadExecutionState`, nothing persisted. |
| File unlocker | "Who is locking this file?" via the Restart Manager API. Shows the process; closing it is the user's call, no forced handle closing. |
| Legacy panels launcher | One screen of shortcuts to classic Control Panel applets (`ncpa.cpl`, `sysdm.cpl`, Device Manager, …). Read-only, good first issue. Inspired by WinUtil. |
| Restore point | "Create a restore point" button, offered before any system change. Inspired by WinUtil. |

## v0.3.0 — system and network

| Item | Notes |
| --- | --- |
| Network & ports | Listening ports with owning process, per-process traffic. Read-only; merges port-inspector and network-monitor-lite. |
| DNS switcher | Pick a resolver per adapter; the previous setting is stored and restorable. Inspired by WinUtil. |
| Repair kit | `sfc /scannow`, `DISM /RestoreHealth`, DNS flush, winsock reset, Windows Update component reset. Each button shows the exact command and asks first. Inspired by WinUtil's fixes tab. |
| Clipboard history+ | Off by default, local only, encrypted with DPAPI, skips password-manager sources. |
| Environment variable editor | PATH split into rows, snapshot before edit, diff on save. |

## Later candidates

| Item | Notes |
| --- | --- |
| Reversible tweaks | Curated Windows annoyance toggles (web results in Start, lock-screen ads, …). Each records the original value, links to Microsoft documentation, offers undo, and suggests a restore point first. No Appx or service removal. WinUtil's MIT-licensed tweak list can seed the catalogue with attribution. Replaces BACKLOG "debloat-lite". |
| Optional Windows features | Turn WSL, Windows Sandbox, Hyper-V and similar on or off through the supported DISM feature API. |
| Power plan switch | List and switch power plans; show the active one. |
| Settings profile export | Export / import WinKnife settings and the tweak state to a file. |
| Context-menu cleaner, big-file archiver, uninstaller+, hosts editor | See BACKLOG Tier A/B. |
| Optional LLM explanations | Opt-in, off by default (ARCHITECTURE §4). Not before v0.2.0. |

## Not planned

From WinUtil and similar tools we deliberately **do not** take: disabling
Windows Update, removing Edge / OneDrive / built-in Appx packages, MicroWin ISO
building, and service-disabling packs. They are hard to undo and can break
updates. Also out: RAM booster, driver updater, registry "optimizer", scheduled
auto-clean, force-closing handles or killing processes without consent, cloud
sync, a plugin marketplace (for now), and installing or upgrading apps
through winget before v1.0: it would make WinKnife a networked installer and
break the simple "no network" promise of the 0.x releases. See BACKLOG non-goals.
