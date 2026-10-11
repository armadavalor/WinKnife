# Changelog

User-visible changes. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/). Development notes
live in [docs/CHANGELOG-DEV.md](docs/CHANGELOG-DEV.md).

## [Unreleased]

## [0.1.0] — unreleased

First public release.

### Added
- **Disk Cleaner** — read-only scan with a reason and risk bucket for every
  finding; selected items move to quarantine; the previous scan is shown as an
  estimate until you scan again.
- **Duplicate Finder** — byte-identical duplicate detection with review before
  quarantining selected copies.
- **Disk map** — folder sizes, drill-down and a treemap.
- **Startup** — review login entries, disable and restore supported entries.
  Only entries whose program is outside protected locations can be rated Safe.
- **Shutdown Timer** — shutdown, restart or sleep with countdown, warnings and
  cancellation.
- **Quarantine** — restore to the original location while retained; age and
  size limits purge old items.
- English and Turkish interface.

### Security
- Content security policy for the app window; unused opener plugin removed.
- Log folder keeps only the newest 14 daily files.

[Unreleased]: https://github.com/armadavalor/WinKnife/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/armadavalor/WinKnife/releases/tag/v0.1.0
