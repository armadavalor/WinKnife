# Changelog

User-visible changes. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/). Development notes
live in [docs/CHANGELOG-DEV.md](docs/CHANGELOG-DEV.md).

## [Unreleased]

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

[Unreleased]: https://github.com/armadavalor/WinKnife/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/armadavalor/WinKnife/releases/tag/v0.2.0
