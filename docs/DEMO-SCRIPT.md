<!-- durum: aktif -->
# Demo GIF script

Two GIFs for the README. Short beats a complete tour: the main GIF proves the promise "analyze first, nothing is deleted without consent, everything is reversible". The second one shows the v0.2.0 tools.

## Recording setup
- 1280×800 window, dark theme, English UI, 100% DPI scaling. Hide the taskbar clock and notifications (Focus Assist on).
- Tool: ScreenToGif (free). 15 fps, then trim the waits in the editor. Target < 5 MB per GIF.
- Prepared data: a throwaway user profile or VM with about 1–3 GB of real junk (browser cache, `%TEMP%`, Windows Update leftovers). No real names in paths, no personal files on screen.
- Before recording: start `winknife.exe` once so the first scan is warm, empty the quarantine, close the other apps.
- Mouse: slow and deliberate, pause ~0.5 s on each button before clicking. No typing on camera except the restore-point description.

## GIF 1 — "Scan → review → clean → undo" (≤ 20 s)

| # | Time | Screen | Action | What the viewer should get |
|---|------|--------|--------|---------------------------|
| 1 | 0–2 s | Dashboard | Hold still | Calm, dark, one app for many jobs |
| 2 | 2–5 s | Cleaner | Click **Scan** and let progress run (cut the middle) | It looks first and deletes nothing |
| 3 | 5–9 s | Cleaner results | Hover a green (Safe) group, open one item's reason, then hover an amber (Needs review) item | Every suggestion says why; risky items are not pre-selected |
| 4 | 9–12 s | Cleaner | Leave only the green groups ticked, click **Clean**, confirm | You decide; the total size freed is visible |
| 5 | 12–16 s | Quarantine drawer | Open the drawer: the items are listed with a date | "Deleted" items went to quarantine, not to oblivion |
| 6 | 16–20 s | Quarantine drawer | Click **Restore** on one item → toast "restored" | Everything is reversible |

End frame: hold for 1 s on the restored toast.

## GIF 2 — "Everyday tools" (v0.2.0, ≤ 20 s, record after the v0.2.0 merge)

| # | Time | Screen | Action |
|---|------|--------|--------|
| 1 | 0–5 s | File unlocker | Drop a `.docx` that is open in Word → row "WINWORD.EXE — holds report.docx" with its reason → **Ask to close** → Word asks to save |
| 2 | 5–9 s | Empty folders | Scan `%USERPROFILE%` → tree of empty folders with reasons; quarantine one |
| 3 | 9–13 s | Keep awake | Click **1h** → countdown starts; click **Stop** |
| 4 | 13–17 s | Startup | Show the "Create a restore point first" link → restore-point page → **Create** (cut the UAC prompt) → "created" |
| 5 | 17–20 s | Second launch | Run `winknife --module treemap` from a shortcut while the app is open → the existing window comes forward on the treemap |

## Avoid on camera
- Red alarm colors, fake "X GB of junk found!" hype, the UAC dialog itself, real user names in paths.
- The Settings page and any debug/dry-run banners.
- Do not clean anything amber or red in the demo, even on a VM: the GIF teaches the behavior we want from users.

## Shot list for README stills (optional)
Cleaner results with reasons expanded · Treemap · Duplicates with "keep newest" · Startup with the reason column · File unlocker result row.
