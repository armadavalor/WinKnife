# WinKnife demo (Remotion)

Renders the README demo GIF/MP4 from real app screenshots. Dev-only: nothing here ships in the app or the Tauri bundle.

```bash
cd demo
npm ci
npm run render:gif   # out/demo.gif (960x540, 15 fps, ~4 MB)
npm run render:mp4   # out/demo.mp4 (1280x720, 30 fps)
npm run studio       # live preview
```

`out/` is gitignored. Copy the approved GIF to `assets/demo-remotion.gif` (or `assets/demo.gif` once the README is switched over).

## Refresh the screenshots (after a UI change)

Stills in `public/`: `scan`, `results`, `confirm`, `restore` (+ `dashboard`, unused), 1280x800 PNG, shown at 1000x625.

1. Needs a **released** build (the workflow installs it): `gh workflow run demo.yml --ref main -f version=X.Y.Z`. It seeds junk into a fake `C:\Users\Demo` profile, so no real names or paths leak.
2. `gh run list --workflow demo.yml`, then `gh run download <id> -n demo-frames` (`NNNNN.png` + `frames.txt`).
3. Pick one frame per scene: Disk Cleaner idle with the Scan button; results with reasons visible; the "Move N items to quarantine?" dialog; the quarantine drawer with the cursor on Restore. Copy over the files in `public/`.
4. Without a release: run a local dev build with `--remote-debugging-port=9222` and drive it with `scripts/demo/record.mjs` against a fake-profile folder.
5. Re-render and eyeball a few frames: `npx remotion still src/index.ts Demo f.png --frame=130`.

Captions and timings live in `src/Demo.tsx` (`SCENES`).

## Licence

Remotion is free for individuals, teams of up to 3 and non-profits (`remotion.config.ts` sets `free-license`). See `docs/LISANSLAR.md`.
