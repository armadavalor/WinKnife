# WinKnife demo (Remotion)

Renders the ~15 s looping README b-roll (`assets/demo/demo.gif`, linked to `assets/demo/demo.mp4`) from real app stills. Dev-only: nothing here ships in the app or the Tauri bundle.

```bash
cd demo
npm ci
npm run render:mp4   # out/demo.mp4 (1280x720, 30 fps, h264)
npm run studio       # live preview
```

The GIF is made from the MP4 with the ffmpeg Remotion bundles (Remotion's own GIF codec gives ~50 MB because the halftone sky moves). This build has no `fps`/`select` filters, so use `-r`:

```powershell
$ff = "node_modules/@remotion/compositor-win32-x64-msvc/ffmpeg.exe"
& $ff -y -i out/demo.mp4 -r 8 -filter_complex "scale=640:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=64:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" -loop 0 out/demo.gif
```

Keep the GIF under ~8 MB. `out/` is gitignored; copy the approved files to `assets/demo/`.

## Scenes

`src/Demo.tsx`: dashboard → Disk Cleaner with reasons → quarantine → restored → disk map → a whip tour of one still per tool (`TOUR_STILLS`) → end card (`KnifeLogo.tsx`, "Fast. Modular. Useful."). Captions, timings and camera moves live in `SHOTS`. `SkyBackdrop.tsx` draws the app's Bliss halftone sky (`sky.ts` mirrors `src/lib/sky.ts`) under everything, on a period that matches the composition so the loop is seamless.

## Refresh the stills (after a UI change)

Stills in `public/broll/`: 2560x1600 PNG with a transparent background (the app's backdrop is hidden while capturing, so they sit over the animated sky).

1. Needs an installer: a published release (`-f version=X.Y.Z`) or a draft release run (`-f release_run=<run id>`).
2. `gh workflow run demo.yml --ref main -f script=broll -f release_run=<id>`. On a throwaway runner it seeds junk into a fake `C:\Users\Demo` profile and a fake `W:` drive and drives the app with `scripts/demo/broll.mjs`, so no real names or paths are in frame. Never capture on a real machine.
3. `gh run download <run> -n demo-frames`, copy the PNGs into `public/broll/`.
4. Check every still for real usernames or paths before committing.
5. Re-render and eyeball a few frames: `npx remotion still src/index.ts Demo f.png --frame=200`.

## Licence

Remotion is free for individuals, teams of up to 3 and non-profits (`remotion.config.ts` sets `free-license`). See `docs/LISANSLAR.md`.
