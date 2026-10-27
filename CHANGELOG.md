# Changelog

User-visible changes. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/). Development notes
live in [docs/CHANGELOG-DEV.md](docs/CHANGELOG-DEV.md).

## [Unreleased]

## [0.5.0] — 2026-10-27

### Added
- **Pinned tools** — the dashboard shows the tools you pin instead of the last
  ones you used. Use "+ Add tool" to pin any tool and × to unpin.
- **Translucency sliders** — Settings → Appearance sets how see-through the
  sidebar and the frosted dashboard panels are (30–100 %).
- Cloud backdrop, frosted sidebar and themed scrollbars; animations are on by
  default on a fresh install.

### Changed
- The animated backdrops use about 4× less CPU per frame and stand still while
  the window is not in focus.

## 0.4.0 — 2026-10-25

### Added
- **Turkish interface** — choose English or Türkçe in Settings; it applies at
  once and follows the Windows language on first launch. Reasons and errors
  produced by the scan engine are still English.
- **Tweaks** — 27 reversible Windows settings (current user only), grouped and
  searchable. The old value is saved before each change and Undo puts it back;
  a changed-behind-your-back setting is flagged with Reapply.
- **Context menu** — lists right-click menu entries with their owner and
  publisher; entries can be disabled for the current user and re-enabled.
- **Hosts editor** — view and edit the hosts file with a backup before saving.
- **Duplicate folders** — folders with identical contents are grouped, with a
  keep rule per group; at least one copy always stays.
- **Settings profile** — export and import your settings as JSON; import shows
  the differences first.
- **Long lists** — search, filter, select all/none/invert and collapsible
  groups on Cleaner, Duplicates, Empty Folders and Startup.
- **Quarantine** — permanently remove single items, a selection or a batch
  after confirmation; locked items stay with the reason.
- **Restart as administrator** — offered when an action is denied for lack of
  rights; only runs when you click it.
- **Startup** — each entry shows its rating and the reason for it.
- **Settings** — untick a tool to take it out of the sidebar; it stays on the
  dashboard and still opens from there. An About card lists what WinKnife never
  does.

### Changed
- New knife logo and background glow; animations can be switched off.
- Restores interrupted by a crash are repaired at next launch.
- Release builds carry a GitHub build attestation; see "Verify this download"
  in the README.

## 0.3.0 — 2026-10-19

### Added
- **Legacy Panels** — open 22 classic Windows tools from one page; unavailable
  tools appear with a disabled button.
- **Appearance** — choose Graphite, Forest or Mist in Settings; the choice is
  saved for the next launch.

### Changed
- **Interface** — clearer tool descriptions and status actions, consistent
  controls and dialogs, and secondary details that expand on demand.
- **Icons** — consistent Lucide icons across the sidebar, cards and controls.
- **Disk Cleaner** — large scans show analysis progress and a Stop button;
  findings load in groups of 200 with a Show more control.

### Fixed
- **Disk Cleaner** — large Temp scans complete faster and can be stopped during
  analysis; lock checks now avoid most Defender-triggering read access when
  cleaning many files.
- **Disk Cleaner** — the quarantine confirmation title now shows the selected
  item count instead of a placeholder.
- **Duplicates and Empty Folders** — Stop cancels the running scan, including
  when pressed before the first progress update.
- **Startup** — disabling an entry no longer stalls on Working or freezes the
  app window.
- **Finding lists** — restored normal-sized chevrons in Cleaner and Duplicates.
- **Quarantine** — a restore or purge refused because another operation is
  running now says so in the drawer instead of failing silently.

## [0.2.0] — 2026-10-13

First public release. (0.1.0 was never tagged; its contents ship here.)

### Added
- **Disk Cleaner** — read-only scan with a reason and risk bucket for every
  finding; selected items move to quarantine; the previous scan is shown as an
  estimate until you scan again. Emptying the Windows recycle bin is the one
  permanent action: it is never pre-selected and needs its own confirmation.
- **Duplicate Finder** — byte-identical duplicate detection with review before
  quarantining selected copies.
- **Disk map** — folder sizes, drill-down and a treemap.
- **Startup** — review login entries, disable and restore supported entries.
  Only entries whose program is outside protected locations can be rated Safe.
- **Shutdown Timer** — shutdown, restart or sleep with countdown, warnings and
  cancellation.
- **Quarantine** — restore to the original location while retained; age and
  size limits purge old items.
- **Empty Folders** — finds folders with no file at any depth, reports only the
  topmost one of each empty tree and moves the selected ones to quarantine.
- **File Unlocker** — shows which program holds a file open (Restart Manager);
  the only action is a polite request to close it.
- **Keep Awake** — keeps the PC (optionally the screen) awake for 1 minute to
  24 hours; nothing is persisted.
- **Restore Point** — shows System Protection status and creates a restore
  point through an elevated helper (UAC prompt).
- A second launch focuses the running window; `--module <id>` opens a tool
  directly.
- English interface (Turkish strings exist for the newer tools but there is no
  language switch yet); info tooltips, card-grid dashboard and compact
  empty states.

### Security
- Content security policy for the app window; unused opener plugin removed.
- Log folder keeps only the newest 14 daily files.

[Unreleased]: https://github.com/armadavalor/WinKnife/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/armadavalor/WinKnife/releases/tag/v0.5.0
[0.2.0]: https://github.com/armadavalor/WinKnife/releases/tag/v0.2.0
