<!-- durum: aktif -->
# Code signing with SignPath Foundation

v0.2.0 shipped unsigned, so Windows SmartScreen holds the installer at
"Windows protected your PC" and winget installs are blocked. This document
describes how WinKnife applies for free open-source code signing from
[SignPath Foundation](https://signpath.org/) and how the release workflow uses
it. The rules below were read from the sources listed at the end on 2026-10-14;
SignPath changes its terms, so re-read them before applying.

Only the NSIS installer (`WinKnife_<version>_x64-setup.exe`) is signed. The
`winknife.exe` inside the installer and in the portable ZIP stay unsigned for
now (see "Known gaps").

## Eligibility checklist

| Requirement (SignPath terms) | WinKnife status |
| --- | --- |
| OSI-approved license for all components, no commercial dual licensing | Met. MIT, no dual licensing. Dependencies are npm/crates under permissive licenses (tracked in `docs/LISANSLAR.md`); re-check before applying. |
| No proprietary / non-open-source components | Met. The WebView2 runtime is a Windows system component, not bundled. |
| No malware / potentially unwanted programs | Met in intent. VirusTotal showed 3/71 generic ML detections on v0.2.0; mention them in the application as heuristics on an unsigned binary. |
| Actively maintained | Met. Releases and commits are recent. |
| Already released in the form to be signed | Met. The v0.2.0 NSIS installer is public. |
| Functionality described on the download page | Met. README.md lists every tool. |
| Own project, own binaries, own build scripts | Met. Built from this repository by `.github/workflows/release.yml`. |
| Not a hacking / vulnerability-scanning tool | Met. WinKnife is a cleaner and system-tools utility. |
| Does not compromise privacy; no undisclosed network traffic | Met. No network calls; the LLM layer is not implemented. |
| No system changes without warnings | Met. Startup edits, recycle-bin emptying and restore points are user-initiated with confirmation dialogs. |
| Uninstall provided with the installer | Met. NSIS uninstaller. |
| Builds are automated on GitHub-hosted runners (origin verification) | Met. The release workflow runs on `windows-latest`. |
| MFA for SignPath and GitHub for every team member | USER ACTION. Enable on GitHub and on the SignPath account. |
| "Code signing policy" published on the home page and release pages | Added to README.md in this change; copy it into release notes (see below). |
| Privacy policy or a statement that no data is transferred | Covered by the policy section; no separate page needed. |
| Team roles (authors, reviewers, approvers) listed | Listed. Single maintainer today, see "Risk". |
| File metadata: product name equals project name, product version constant per build | Product name is `WinKnife` from `tauri.conf.json`; enforce it in the SignPath artifact configuration. |

**Blockers found:** none known.

**Risk:** the project has one maintainer, who would be author, reviewer and
approver. SignPath requires the team to organise these roles and approvers to be
trusted by the team; the published terms do not exclude a one-person team, but
acceptance is discretionary. A second trusted reviewer or approver would
strengthen the application.

## Application text (paste into the SignPath form)

> **Project:** WinKnife - https://github.com/armadavalor/WinKnife
>
> **License:** MIT (OSI approved), no commercial dual licensing and no
> proprietary components.
>
> **Description:** WinKnife is an open-source Windows utility that combines disk
> cleaning, duplicate and empty-folder finding, a disk map, startup management,
> a file unlocker, keep-awake, restore points and a shutdown timer. Its core
> promise is "analyze first, never delete without consent, everything
> reversible": cleaned files go to a local quarantine and can be restored,
> protected system paths are refused in code, and every recommendation carries a
> human-readable reason. The app makes no network calls. Built with Tauri v2
> (Rust) and Svelte.
>
> **Released:** v0.2.0 is public on GitHub Releases as an NSIS installer and a
> portable ZIP. The installer is the artifact to be signed. It is unsigned
> today, so SmartScreen blocks it and our winget submission is stalled.
>
> **Build:** every release is built by the GitHub Actions workflow
> `.github/workflows/release.yml` on GitHub-hosted `windows-latest` runners from
> a version tag; format, lint, tests and UI tests run before the build. The
> workflow uploads the unsigned installer as a GitHub artifact and submits it
> with `signpath/github-action-submit-signing-request`.
>
> **Team:** single maintainer (GitHub: armadavalor) acting as author, reviewer
> and approver; MFA is enabled on GitHub and SignPath. External pull requests
> are reviewed by the maintainer before merge.
>
> **Code signing policy:** https://github.com/armadavalor/WinKnife#code-signing-policy
>
> **Privacy:** WinKnife does not transfer information to any networked system.
>
> **Reputation:** public repository, MIT license, CI, tagged releases, winget
> manifest submitted to microsoft/winget-pkgs.

Adjust the Reputation line to what is true on the day you apply (stars, winget
status, VirusTotal links).

## Code signing policy (published text)

The text lives in README.md under "Code signing policy" and is repeated here as
the reference. SignPath requires the heading "Code signing policy" on the
project home page and on download/release pages, so paste the same section into
each GitHub release's notes once signing is live.

```markdown
## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io/),
certificate by [SignPath Foundation](https://signpath.org/).

Only the installer built from this repository by the GitHub Actions workflow
`.github/workflows/release.yml` on GitHub-hosted runners is submitted for
signing, and every signing request is approved manually. Releases up to and
including v0.2.0 are unsigned.

| Role | Members |
| --- | --- |
| Authors (may change source without further review) | [@armadavalor](https://github.com/armadavalor) |
| Reviewers (review every change from non-committers) | [@armadavalor](https://github.com/armadavalor) |
| Approvers (approve each signing request) | [@armadavalor](https://github.com/armadavalor) |

Privacy: WinKnife does not transfer any information to other networked systems.
```

## Steps the repository owner must do

None of these were done by the automation that prepared this change.

1. **Enable MFA** on the GitHub account that owns the repository and on every
   other person who will have write access.
2. **Merge this PR** so the policy section is public, then check that
   `#code-signing-policy` resolves on the repository home page.
3. **Apply:** open https://signpath.org/apply.html, read https://signpath.org/terms,
   and submit the form with the application text above. Acceptance is
   discretionary; expect a wait.
4. **After acceptance**, in your SignPath organization:
   1. Create the SignPath user with MFA enabled.
   2. Add the predefined trusted build system **GitHub.com** to the
      organization and link it to the WinKnife project.
   3. Create an **artifact configuration** (slug of your choice) like:

      ```xml
      <artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
        <zip-file>
          <pe-file path="*-setup.exe" product-name="WinKnife">
            <authenticode-sign/>
          </pe-file>
        </zip-file>
      </artifact-configuration>
      ```

      The uploaded artifact is a ZIP, hence the `zip-file` root. If SignPath
      asks for NSIS-specific handling for installers, follow what it shows when
      you create the configuration (not covered by the pages read here).
   4. Create the **release-signing policy** with manual approval, yourself as
      approver, and origin verification on.
   5. Create an **API token** for a user with submitter permission on that
      project and policy.
5. **Install the SignPath GitHub App** (https://github.com/apps/signpath) on the
   repository; SignPath lists it as optional but required for audit-log
   evaluation.
6. **Add to the GitHub repository** (Settings, Secrets and variables, Actions):
   - Secret `SIGNPATH_API_TOKEN`: the API token from step 4.5.
   - Variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`,
     `SIGNPATH_SIGNING_POLICY_SLUG`, and optionally
     `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` (empty means the project default).
7. **Cut a test release** (`workflow_dispatch` or a tag). Approve the request in
   SignPath when it arrives; the workflow waits up to two hours.
8. **Publish** the draft release, copy the "Code signing policy" section into
   its notes, update the winget manifest hash (the signed installer has a new
   SHA-256), and tell the winget-pkgs reviewers the installer is now signed.

## How the release workflow behaves

| State | Behavior |
| --- | --- |
| Secret `SIGNPATH_API_TOKEN` absent (today) | The three signing steps are skipped. The workflow is identical to before: unsigned installer, ZIP, `SHA256SUMS.txt`. |
| Secret present, variables correct | After `tauri build`, the unsigned setup.exe is uploaded as artifact `winknife-unsigned-installer`, submitted to SignPath, and the workflow waits for approval. The signed file overwrites the installer in `src-tauri/target/release/bundle/nsis/` after a check that its Authenticode status is `Valid`. |
| Packaging | `scripts/package-release.ps1` runs after signing and copies the installer from that folder, so the released setup.exe and its entry in `SHA256SUMS.txt` are the signed ones. No checksum regeneration is needed. |
| Request rejected, timed out, or wrong variables | The signing step fails, so the run fails and no draft release is created. Remove the secret to release unsigned on purpose. |

`workflow_dispatch` runs also sign when the secret exists. Deny any request in
SignPath that you did not intend to release.

## Known gaps

- `winknife.exe` inside the installer and the portable ZIP is not signed.
  SmartScreen judges the installer first, but signing the app binary too would
  need a two-pass build (sign `winknife.exe`, then bundle) and its own entry in
  the artifact configuration. SignPath asks projects to maximise signing
  coverage, so treat this as a follow-up.
- SmartScreen reputation for a new certificate builds over time; the first
  signed releases may still warn, though a valid signature replaces
  "Unknown publisher".
- `actionlint` was not available when the workflow was edited; the YAML was
  parsed but not linted.
- README.tr.md does not yet carry the policy section.

## Sources

- Terms (eligibility, roles, policy wording): https://signpath.org/terms
- Apply page: https://signpath.org/apply.html
- GitHub integration, inputs, token, trusted build system: https://docs.signpath.io/trusted-build-systems/github
- Action repository (`action.yml`, v3): https://github.com/signpath/github-action-submit-signing-request
- Artifact configuration syntax: https://docs.signpath.io/artifact-configuration/syntax
- Origin verification: https://docs.signpath.io/origin-verification
