# Phase 7: repair kit (C6)

Module `repair`: buttons for Windows' own SFC and DISM. Checklist and result.

- [x] Five tools, each showing its exact command line: `DISM /Online /Cleanup-Image /CheckHealth`, `/ScanHealth`, `/RestoreHealth`, `sfc /scannow`, `/AnalyzeComponentStore`. Nothing runs until a click.
- [x] RestoreHealth and sfc go behind ConfirmDialog with a plain explanation; the other three run directly.
- [x] Unelevated: `needs_admin` (K-021 banner), never self-elevate.
- [x] Live output via `repair:output`; sfc's UTF-16 output decoded; DISM progress lines (carriage returns) replace each other and are throttled.
- [x] One run at a time (`Operation::Repair`, `busy`); Cancel via `CancellationToken` kills the child. `CREATE_NO_WINDOW`.
- [x] One-line verdict from output patterns; exit-code fallback (DISM 3010 = restart, 0x800F081F = source missing); otherwise "finished, see log".
- [x] Full log per run in `log_dir` (`repair-<tool>-<stamp>.log`), "Open log" reveals it in Explorer.
- [x] Unarmed process (tests, dev, e2e, `--dry-run`): fake runner, nothing spawned.

Deferred: `StartComponentCleanup` (and `/ResetBase`), see DECISIONS-NEEDED.
