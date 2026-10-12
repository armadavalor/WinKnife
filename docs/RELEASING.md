# Release checklist

The current work prepares v0.1.0. It does not declare Phase 4 complete.
The phase brief also includes optional LLM adapters, credential storage and
privacy controls; those have not been implemented. Do not advertise them.

## Before creating a tag

1. Review changes and synchronize versions in package.json, package-lock.json,
   src-tauri/Cargo.toml, src-tauri/Cargo.lock and tauri.conf.json.
2. Run the checks in README.md. Link safety tests must actually run: enable
   Windows Developer Mode or use an elevated test session. A local run with
   WINKNIFE_ALLOW_SKIP_LINK_TESTS=1 is useful to test the rest of the suite but
   is not release acceptance. Never set that option in CI.
3. Run `npm run tauri build -- --bundles nsis`, then
   `./scripts/package-release.ps1 -Version 0.1.0` in PowerShell at the repo root.
   The portable ZIP includes rules and license files; an exe alone is insufficient.
   Review/archive an existing release-artifacts directory before packaging again.
4. Complete the fresh-VM checks below. Record Windows/WebView2 versions,
   artifact SHA-256 values, findings and date. Do not use a personal machine's
   files or startup entries for destructive tests.
5. Record a short scan → review → clean → restore demo using disposable fixtures.
   The planned hero GIF has not been produced yet; do not substitute a fake demo.
6. Upload the exact release artifacts to VirusTotal, record the result URLs and
   investigate detections. This is a manual, explicit publication step. A scan
   link is evidence for that artifact hash, not a safety guarantee.
7. Review README links, known limitations and unsigned-installer messaging.
   No donation URL is included until the maintainer provides one.

## Fresh Windows VM acceptance

- Snapshot the VM. Install the NSIS package as a normal user; launch it with
  the required WebView2 runtime available. Exercise the missing-runtime path too.
- Create disposable old temp files. Scan, review reasons, clean, restore and
  compare file contents. Confirm restored files return to their original paths.
- Check protected-path refusals, no network requirement and cancelled scans.
- Exercise startup enable/disable on a disposable entry and restore it.
- Use `--dry-run` for timer checks. This flag disables power actions only;
  it does not prevent file cleanup, startup changes or quarantine purge.
- Restart the app and confirm cleaner history, settings and quarantine records.
- Check the seven pages at 1100×720 and 880×560, including populated lists,
  confirmation dialogs and quarantine drawer. Record any horizontal overflow.
- Uninstall and verify the app is removed. Record what happens to app data;
  do not delete retained quarantine files as part of an “uninstall cleanup”.
- Extract the portable ZIP into a new directory and repeat launch/scan/restore
  with no access to the development checkout. Verify bundled rules are found.

## CI and publication

The Release candidate workflow can run manually to produce reviewable artifacts.
A pushed `v*` tag must match package versions and runs all checks before building.
It attaches the NSIS installer, portable ZIP and SHA256SUMS.txt to a **draft**
GitHub Release. A failure must be investigated; do not force a tag or overwrite
an existing release to make a rerun appear clean.

The maintainer reviews the draft, adds fresh-VM evidence, known limitations,
VirusTotal links and checksums to the notes, then explicitly publishes it.
No tag, push, release publication or external file upload is performed by the
local packaging script.
