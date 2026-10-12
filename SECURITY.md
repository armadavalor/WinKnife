# Security Policy

WinKnife moves, quarantines and restores files and edits some Windows settings,
so bugs can cause data loss. We treat the following as security issues:

- a file deleted permanently without going through quarantine and consent,
- a protected path (`C:\Windows`, `C:\Program Files*`, profile roots, …)
  receiving a Safe rating or a cleanup action,
- symlink, junction or reparse-point tricks that make an action touch a path
  other than the one the user reviewed,
- a restore that overwrites or lands somewhere other than the original location,
- any network access without user opt-in,
- code execution or privilege escalation through the app or its installer.

## Supported versions

Only the latest release receives fixes. Before v1.0, that means the newest
`0.x` tag.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private reporting:
**Security → Report a vulnerability** on
<https://github.com/armadavalor/WinKnife/security/advisories/new>.

Include the WinKnife version, Windows version, steps to reproduce and, if you
can, the relevant part of the log folder shown in Settings.

You should get an acknowledgement within 7 days. Fixes for confirmed issues are
released as soon as practical and credited in the release notes unless you ask
otherwise.
