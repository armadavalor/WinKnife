<!-- durum: aktif -->
# Five-person cold first-run test

Five people who have never seen WinKnife try v0.3.0 from the GitHub page
onward, without help. The goal is to find where real users stop or make a
wrong choice before the public launch ([POSTS.md](POSTS.md)). Five sessions
are evidence for finding problems. They are not a safety guarantee.

Kit: [`scripts/coldtest/`](../../scripts/coldtest/). Scores:
[`cold-test-scores.csv`](cold-test-scores.csv).

## Participants

| Code | Profile | Recruit from |
| --- | --- | --- |
| P1, P2 | Ordinary Windows user: uses the PC daily, never edits the registry | Friends and family who have not heard about the project |
| P3, P4 | Technically comfortable: installs software from GitHub, knows what Temp is | Developer or gamer acquaintances |
| P5 | IT or support practitioner | Someone who supports other people's PCs |

Nobody who has seen the app, the README or a demo. Each session takes about
25 minutes plus 5 minutes of setup. Remote sessions are fine: the
participant controls the moderator's screen, or the session runs on a
machine they use.

## Machine: one fresh Windows Sandbox per participant

The host needs Windows 10/11 Pro or Enterprise with the **Windows Sandbox**
optional feature turned on. Run from a checkout of this repository:

```powershell
.\scripts\coldtest\Start-ColdTest.ps1 -Participant P1
```

What happens:

1. `Start-ColdTest.ps1` writes a `.wsb` file to `%TEMP%` with the host paths
   filled in and opens it. Results go to
   `%USERPROFILE%\WinKnife-coldtest-results` on the host (`-ResultsDir`
   to change it).
2. Inside the sandbox, `coldtest-bootstrap.ps1` runs at logon (about 1–2
   minutes, minimised window):
   - runs `scripts/qa/New-DirtyProfile.ps1` on the sandbox profile with 5,000
     temp files, 30 duplicate sets and 40 empty folders. The profile contains
     browser caches, old installers, duplicates in Documents, Pictures and
     Downloads, empty project trees, Unicode and very long paths, a junction
     loop, a junction to `C:\Windows`, a denied folder and a locked file;
   - adds eight important-looking files that no tool may ever offer (tax
     return, thesis, family photos, a voice memo). They are 400 days old on
     purpose;
   - hashes every file the fixture created into `<P>-manifest.json`. Each file
     is classed as **keep** or **offerable**. Keep covers the eight files
     above, the recent `Downloads\this-week-report.pdf`, the fresh Temp file
     and the `main.rs` inside the empty-folder tree;
   - opens Edge on <https://github.com/armadavalor/WinKnife>.
3. WinKnife is **not** installed. Finding, downloading and installing it is
   task 2.

Closing the sandbox throws everything away, so the next participant starts
from a clean machine. Only one sandbox can run at a time. SmartScreen and
Defender stay on.

The two script folders are mapped read-only at `C:\coldtest\`. The results
folder is the only writable mapping. The participant has no reason to open
`C:\coldtest`. If they do, note it, but don't stop them.

**If the sandbox hides the SmartScreen warning** (the sandbox can behave
differently from a real install), run at least one session (preferably P1
or P2) on a fresh Windows VM snapshot. On that VM, copy `scripts\qa` and
`scripts\coldtest` to `C:\coldtest\qa` and `C:\coldtest\scripts`, then run
the bootstrap as a throwaway local user named `WDAGUtilityAccount`. The name
check is the guard that keeps the fixture off real profiles.

## Before each session (moderator)

- [ ] Consent: the screen and microphone are recorded, the recording is used
      only to improve WinKnife, and it is deleted after analysis. They may
      stop at any time.
- [ ] OBS records the sandbox window and the microphone.
- [ ] The bootstrap window says `Ready. Hand over to the participant.` If
      it says `BOOTSTRAP FAILED`, close the sandbox and start again.
- [ ] Ask the "before" question (below) and write down the answer.

## Script

Read this out, word for word:

> "This is a fresh test computer, and nothing on it matters. Use it as you
> normally would. Please think aloud: say what you expect, what worries you,
> and why you choose each action. I can't help you or answer questions
> about the program while you work. You can stop at any point, and stopping
> is a useful result for me."

**No help rule.** Do not explain, point or hint. If asked, say "What would
you do if I weren't here?" After 3 minutes stuck on one task, say "Let's
move on". The task then counts as failed with help (`assisted`).

Give the tasks one at a time, on paper or in chat:

| # | Task given to the participant | Success when |
| --- | --- | --- |
| T1 | "Read this page and decide whether you would try this program." | They explain what it does, notice it is unsigned and say what happens to cleaned files (quarantine, restorable while retained) |
| T2 | "Install it." | WinKnife opens without help, or they clearly say why they stopped (SmartScreen, antivirus, distrust) |
| T3 | "Find out what could be cleaned, without cleaning anything yet." | A scan finishes. The verifier later shows no changes made before T5 |
| T4 | "Pick some things you'd clean, then change your mind and cancel." | They can say why items were suggested. Cancel causes no change |
| T5 | "Now clean up the browser caches and old installers." | Only what they approved changes. No keep file is offered or touched |
| T6 | "Close the program, open it again and get one of the cleaned things back." | They find quarantine without help. The restored file matches its hash |
| T7 | "In your own words, what happened to the files you cleaned?" | They tell apart quarantined (restorable, still uses disk until purged), freed and permanently deleted (recycle bin) |

Note T5 deliberately names categories, not paths. Whatever they pick inside
those categories counts as approved. Anything outside them that they also
ticked is approved too. Record it, it is a selection finding, not a failure.

## After each session

1. **Inside the sandbox**, before closing it, open PowerShell and run:

   ```powershell
   C:\coldtest\scripts\Test-ColdTestResult.ps1 -Participant P1
   ```

   It compares every fixture file with the manifest and writes
   `<P>-verify.txt` and `<P>-verify.json` to the results folder:
   - `unchanged`: same path, same SHA-256 (a correct restore also looks like this);
   - `missing`: gone from its path (quarantined or deleted);
   - `changed`: content differs. Always a failure;
   - `unreadable`: locked or denied, at setup or now.

   **FAIL** when a keep file is missing or changed, or any file's content
   changed. The missing offerable files are grouped by folder. Check each
   group against an approval in the recording.
2. Ask the "after" question and the open questions.
3. Fill one row of `cold-test-scores.csv`.
4. Close the sandbox.

Questions:

- **Before T1** (1–5): "How comfortable are you letting a cleanup program
  delete files on your own PC?"
- **After T7** (1–5): the same question, about WinKnife.
- "What almost made you stop?" "What would you need before running this on
  your own PC?" "Anything that surprised you?"

## Scoring

One row per participant in [`cold-test-scores.csv`](cold-test-scores.csv).

| Column | Values |
| --- | --- |
| `t1`…`t7` | `pass`, `assisted`, `fail`, `not_reached` |
| `t1_min`…`t7_min` | minutes, one decimal |
| `stopped_at` | task where they gave up, or empty |
| `stop_reason` | `smartscreen`, `antivirus`, `distrust`, `confusing_ui`, `error`, `time`, other short text |
| `warnings_seen` | which warnings appeared: `smartscreen`, `defender`, `edge_download`, `none` |
| `wrong_selections` | items they ticked and later said they did not mean to |
| `unexplained_errors` | count of errors they could not understand |
| `confidence_before`, `confidence_after` | 1–5 |
| `verify_verdict` | `PASS` or `FAIL` from the verifier |
| `restore_ok` | `yes`, `no`, `not_reached` |
| `unapproved_changes` | missing offerable files with no matching approval in the recording |
| `notes` | one line; details go in the session notes |

## Launch gate

All four must hold before the Show HN post:

1. At least **4 of 5** finish T3 and T5 with `pass` (unaided).
2. Every participant who reached T6 has `restore_ok = yes`.
3. **Zero** `unapproved_changes` and every `verify_verdict` is `PASS`.
4. No restored file has a different hash. One corrupt restore, or one keep
   file touched, blocks the launch until it is fixed and retested with a new
   participant.

If the gate fails because of T1 or T2 (people stop at the warning), the
launch can still go ahead. Fix the README and release-page wording first and
mention it in the post. Gates 2–4 are about safety and have no exceptions.

Every problem found gets a GitHub issue labelled `cold-test`. The issue
contains the participant code, never a name.

## Privacy

Recordings and notes stay offline, are keyed by participant code and are
deleted once the issues are filed. Nothing from a session is posted
publicly without the participant's written consent.
