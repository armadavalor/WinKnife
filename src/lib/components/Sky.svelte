<script lang="ts">
  /**
   * Procedural "Bliss" backdrop: clouds over a green hill, drawn as a halftone
   * dot raster or as ASCII. One canvas, 2D calls only (no image loads, so the
   * production CSP is untouched). Capped at ~20 fps, paused while the window is
   * hidden, and a single still frame when animations are off.
   */
  import { onMount } from "svelte";
  import { motion } from "$lib/stores/motion.svelte";
  import { rampChar, toneAt, type CellKind } from "$lib/sky";

  let { mode }: { mode: "halftone" | "ascii" } = $props();

  const CELL_DOTS = 12; // CSS pixels per grid cell
  const CELL_ASCII = 14; // fewer, larger glyphs: drawImage per cell dominates the cost
  const FRAME_MS = 1000 / 20; // clouds drift slowly; 20 fps reads smooth and halves the cost
  const STILL_T = 40; // where the clouds sit in the still frame
  // Clouds carry the picture; the sky between them stays quiet so cards read on top.
  const ALPHA: Record<CellKind, number> = { sky: 0.3, cloud: 0.6, hill: 0.5 };
  // Thin glyphs carry less ink than dots, so ASCII needs more strength to read.
  const ASCII_BOOST = 1.5;

  interface Palette {
    ground: string;
    sky: string;
    cloud: string;
    hill: string;
  }
  const DARK: Palette = { ground: "#0a1424", sky: "#2f5f9c", cloud: "#f1e8cf", hill: "#4c9a47" };
  const LIGHT: Palette = { ground: "#d6e8f7", sky: "#6aa6dc", cloud: "#ffffff", hill: "#4f9f4b" };

  let canvas: HTMLCanvasElement;
  const cellSize = (): number => (mode === "ascii" ? CELL_ASCII : CELL_DOTS);
  let ctx: CanvasRenderingContext2D | null = null;
  let width = 0;
  let height = 0;
  let dpr = 1;
  let palette = DARK;
  let rafId = 0;
  let last = 0;
  let start = 0;
  // ASCII glyphs are pre-rendered once per palette/size: fillText per cell is ~15x slower than drawImage.
  let atlas: Map<string, HTMLCanvasElement> | null = null;

  function glyph(kind: CellKind, ch: string, cell: number): HTMLCanvasElement {
    if (!atlas) atlas = new Map();
    const key = kind + ch;
    let g = atlas.get(key);
    if (!g) {
      g = document.createElement("canvas");
      g.width = g.height = Math.round(cell * dpr);
      const gc = g.getContext("2d");
      if (gc) {
        gc.scale(dpr, dpr);
        gc.fillStyle = palette[kind];
        gc.font = `${cell}px Consolas, "Courier New", monospace`;
        gc.textAlign = "center";
        gc.textBaseline = "middle";
        gc.fillText(ch, cell / 2, cell / 2);
      }
      atlas.set(key, g);
    }
    return g;
  }

  function readPalette(): void {
    palette = document.documentElement.dataset.theme === "mist" ? LIGHT : DARK;
    atlas = null;
  }

  function draw(t: number): void {
    if (!ctx || width === 0) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.globalAlpha = 1;
    ctx.fillStyle = palette.ground;
    ctx.fillRect(0, 0, width, height);
    const boost = mode === "ascii" ? ASCII_BOOST : 1;

    const cell = cellSize();
    const cols = Math.ceil(width / cell);
    const rows = Math.ceil(height / cell);
    const aspect = width / height;
    const ascii = mode === "ascii";
    const color: Record<CellKind, string> = {
      sky: palette.sky,
      cloud: palette.cloud,
      hill: palette.hill,
    };
    for (let r = 0; r < rows; r++) {
      const ny = (r + 0.5) / rows;
      for (let c = 0; c < cols; c++) {
        const tone = toneAt((c + 0.5) / cols, ny, aspect, t);
        if (tone.b <= 0.02) continue;
        ctx.globalAlpha = Math.min(1, ALPHA[tone.kind] * boost);
        const cx = c * cell + cell / 2;
        const cy = r * cell + cell / 2;
        if (ascii) {
          const ch = rampChar(tone.b);
          if (ch !== " ") ctx.drawImage(glyph(tone.kind, ch, cell), c * cell, r * cell, cell, cell);
        } else {
          ctx.fillStyle = color[tone.kind];
          ctx.beginPath();
          ctx.arc(cx, cy, tone.b * cell * 0.48, 0, Math.PI * 2);
          ctx.fill();
        }
      }
    }
  }

  function frame(now: number): void {
    rafId = requestAnimationFrame(frame);
    if (now - last < FRAME_MS) return;
    last = now;
    draw((now - start) / 1000);
  }

  function stop(): void {
    cancelAnimationFrame(rafId);
    rafId = 0;
  }

  /** (Re)start the loop, or paint one still frame. */
  function redraw(): void {
    stop();
    readPalette();
    if (motion.enabled && !document.hidden) {
      start = performance.now() - STILL_T * 1000;
      rafId = requestAnimationFrame(frame);
    } else {
      draw(STILL_T);
    }
  }

  function resize(): void {
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    width = canvas.clientWidth;
    height = canvas.clientHeight;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    redraw();
  }

  // Re-run when the mode or the animations switch changes.
  $effect(() => {
    void mode;
    void motion.enabled;
    if (ctx) redraw();
  });

  onMount(() => {
    ctx = canvas.getContext("2d");
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    const themeWatch = new MutationObserver(redraw);
    themeWatch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    document.addEventListener("visibilitychange", redraw);
    return () => {
      stop();
      observer.disconnect();
      themeWatch.disconnect();
      document.removeEventListener("visibilitychange", redraw);
    };
  });
</script>

<canvas bind:this={canvas} class="pointer-events-none fixed inset-0 -z-10 h-full w-full" aria-hidden="true"
></canvas>
