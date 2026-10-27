import {useLayoutEffect, useRef} from "react";
import {AbsoluteFill, useCurrentFrame, useVideoConfig} from "remotion";
import {toneAt, type CellKind} from "./sky";

// The app's halftone "Bliss" backdrop (src/lib/components/Sky.svelte, dark
// palette), driven by the frame instead of the clock.
const CELL = 12;
const ALPHA: Record<CellKind, number> = {sky: 0.3, cloud: 0.6, hill: 0.5};
const GROUND = "#0a1424";
const COLOR: Record<CellKind, string> = {sky: "#2f5f9c", cloud: "#f1e8cf", hill: "#4c9a47"};

export const Sky = ({t, opacity = 1}: {t: number; opacity?: number}) => {
  const ref = useRef<HTMLCanvasElement>(null);
  const {width, height} = useVideoConfig();
  useLayoutEffect(() => {
    const ctx = ref.current?.getContext("2d");
    if (!ctx) return;
    ctx.globalAlpha = 1;
    ctx.fillStyle = GROUND;
    ctx.fillRect(0, 0, width, height);
    const cols = Math.ceil(width / CELL);
    const rows = Math.ceil(height / CELL);
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const tone = toneAt((c + 0.5) / cols, (r + 0.5) / rows, width / height, t);
        if (tone.b <= 0.02) continue;
        ctx.globalAlpha = ALPHA[tone.kind];
        ctx.fillStyle = COLOR[tone.kind];
        ctx.beginPath();
        ctx.arc(c * CELL + CELL / 2, r * CELL + CELL / 2, tone.b * CELL * 0.48, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }, [t, width, height]);
  return (
    <AbsoluteFill style={{opacity}}>
      <canvas ref={ref} width={width} height={height} />
    </AbsoluteFill>
  );
};

// Where the clouds start, and how much faster than the app they drift so the
// motion reads in a short clip.
const T0 = 40;
const SPEED = 2;
const BLEND = 30;

// The sky at frame f, crossfaded over the last BLEND frames into the sky of
// frame 0 so the loop has no jump.
export const LoopingSky = ({duration}: {duration: number}) => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const at = (frame: number) => T0 + (frame / fps) * SPEED;
  const k = Math.max(0, (f - (duration - BLEND)) / BLEND);
  return (
    <AbsoluteFill>
      <Sky t={at(f)} />
      {k > 0 ? <Sky t={at(f - duration)} opacity={k} /> : null}
    </AbsoluteFill>
  );
};
