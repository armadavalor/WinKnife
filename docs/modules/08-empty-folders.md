<!-- durum: aktif -->
# Module: empty-folders (Phase 5)

Purpose: find folders that contain no files at all and move them to quarantine. Uses the full Scan → Review → Execute pattern; good first-issue material afterwards.

## Definition of "empty"
A folder is empty when it contains **no files at any depth** (it may contain other empty folders). Report only the **topmost** empty folder of each empty subtree, with a count of nested empty folders in `meta`. Any file, including hidden/system ones like `desktop.ini` or `Thumbs.db`, makes a folder non-empty. Strict on purpose; revisit only via DECISIONS-NEEDED.

## Scan
- Root: user picks a folder; default suggestion = `%USERPROFILE%`. A drive root is allowed as a scan root but is itself never a finding.
- Shared scanner, cancellable, progress events. Never traverse reparse points (junctions, symlinks, OneDrive/cloud placeholders: `FILE_ATTRIBUTE_REPARSE_POINT`, `RECALL_ON_OPEN`, `RECALL_ON_DATA_ACCESS`); a folder that **is** one is not a finding.
- Read errors (access denied) make that folder **not** empty (unknown ≠ empty) and are counted in the scan summary.

## Analyze (brain)
- Hard gates first: protected paths and anything at or above a protected path → `DoNotDelete`. Folders with `FILE_ATTRIBUTE_SYSTEM` → `DoNotDelete` with reason.
- New rule pack `rules/empty-folders.json` (schema-valid, see RULES-FORMAT.md):
  - `%TEMP%` / `%LOCALAPPDATA%\Temp` subfolders older than 7 days → Safe.
  - `%APPDATA%`, `%LOCALAPPDATA%`, `%PROGRAMDATA%` subfolders → NeedsReview, ceiling NeedsReview ("an app may expect this folder to exist").
  - Known shell folders and their direct children (Desktop, Documents, Downloads, Pictures, Music, Videos, OneDrive root) → the shell folder itself `DoNotDelete`; empty children NeedsReview.
  - Everything else → NeedsReview, ceiling NeedsReview.
- Every recommendation has a reason; savings are 0 bytes, so the UI shows a folder count instead of GB for this module.

## Execute
`Action::Quarantine` through the executor; quarantine already handles directories. Re-check emptiness at execute time (a file may have appeared); if no longer empty → skipped with reason, never moved.

## Acceptance
- Unit, tempdir fixtures: nested empty tree reports only the top; one hidden file breaks emptiness; junction not followed and not reported; access-denied subtree treated as not empty; protected path gate; rule pack bucket table.
- Round trip: scan → quarantine → restore puts the folder tree back.
