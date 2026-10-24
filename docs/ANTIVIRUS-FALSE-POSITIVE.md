# If your antivirus flags WinKnife

WinKnife releases are **unsigned** (see [signing notes](SIGNING.md)). Some antivirus engines and Windows SmartScreen treat any new, unsigned executable with little reputation as suspicious. That is the most likely reason you see a warning. We cannot promise it is the only one, so check before you trust it.

## Why it can happen

- No code-signing certificate, and few downloads so far, so reputation systems have nothing to go on.
- WinKnife does things cleaners legitimately do and malware also does: it enumerates files, moves them, reads startup entries, can ask a program to close, and can request a restore point (with a UAC prompt). Heuristic engines react to that pattern.
- The installer is an NSIS package, a format that is often flagged by weak heuristics.

## Check it yourself first

1. Download only from the [releases page](https://github.com/armadavalor/WinKnife/releases).
2. Compare the SHA-256 with `SHA256SUMS.txt` and, if you have the GitHub CLI, run `gh attestation verify` (steps in the README, "Verify this download").
3. If the hash does not match, delete the file and do not run it. Open an issue.
4. The source is public and MIT-licensed; you can build it yourself (README, "Build and run").

A matching hash and attestation show the file is the one this repository built. They do not prove the program is free of bugs.

## Report a false positive

- **Microsoft Defender / SmartScreen:** submit the file at <https://www.microsoft.com/en-us/wdsi/filesubmission> as "Software developer", choose "Incorrectly detected as malware", and include the release link and its SHA-256.
- **Other vendors:** use the vendor's false-positive form; most list it on their website.
- **VirusTotal:** upload the file to see which engines flag it, and link that result in your report. A few detections out of many engines is common for unsigned builds.

Please also open an issue on the repository with the engine name, detection name and file hash. It helps us follow up.

## What we do not claim

We do not claim that every detection is wrong. If a major engine flags a file whose hash matches the release, tell us and we will look at it.
