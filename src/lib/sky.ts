/**
 * Procedural "Bliss"-style sky: drifting clouds over a low round hill. Pure
 * maths, no drawing, so it is testable and the canvas component stays thin.
 *
 * Everything is generated here from a hash; there is no image asset, so no
 * licence to track. Clouds are value-noise fbm that is periodic along x, so a
 * cloud leaving one edge re-enters at the other with no seam.
 */
export const BACKDROPS = ["halftone", "ascii", "light", "off"] as const;
export type Backdrop = (typeof BACKDROPS)[number];

export function isBackdrop(value: unknown): value is Backdrop {
  return typeof value === "string" && (BACKDROPS as readonly string[]).includes(value);
}

/** Characters from empty to dense; a cell's brightness picks one. */
export const ASCII_RAMP = " .:-=+*#%@";

/** The lattice repeats every this many cells along x, so drift loops. */
export const PERIOD = 12;
const OCTAVES = 4;

export type CellKind = "sky" | "cloud" | "hill";
export interface Tone {
  kind: CellKind;
  /** 0 (nothing) to 1 (full dot / densest glyph). */
  b: number;
}

/** The whole sky repeats after this many seconds, so the drift loops. */
export const LOOP_SECONDS = 90;

interface Puff {
  /** Offset from the bank centre (fraction of the width), circle centre height and radius (fractions of the height). */
  dx: number;
  cy: number;
  r: number;
}
interface Bank {
  cx: number;
  base: number;
  /** Half width / height of the whole bank, for a cheap reject. */
  hw: number;
  hh: number;
  puffs: Puff[];
}
interface Layer {
  /** Whole screen widths scrolled per loop. */
  laps: number;
  banks: Bank[];
}

function hash(ix: number, iy: number): number {
  let h = Math.imul(ix, 374761393) ^ Math.imul(iy, 668265263);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
}

function smooth(t: number): number {
  return t * t * (3 - 2 * t);
}

function mod(n: number, m: number): number {
  return ((n % m) + m) % m;
}

/** Value noise in [0,1], periodic along x with the given period. */
function noise(x: number, y: number, period: number): number {
  const x0 = Math.floor(x);
  const y0 = Math.floor(y);
  const fx = smooth(x - x0);
  const fy = smooth(y - y0);
  const a = hash(mod(x0, period), y0);
  const b = hash(mod(x0 + 1, period), y0);
  const c = hash(mod(x0, period), y0 + 1);
  const d = hash(mod(x0 + 1, period), y0 + 1);
  return a + (b - a) * fx + (c - a) * fy + (a - b - c + d) * fx * fy;
}

/** Cloud density in [0,1]; repeats exactly every PERIOD along u. */
export function cloudDensity(u: number, v: number): number {
  let sum = 0;
  let amp = 0.5;
  let freq = 1;
  for (let o = 0; o < OCTAVES; o++) {
    sum += amp * noise(u * freq, v * freq, PERIOD * freq);
    amp *= 0.5;
    freq *= 2;
  }
  return sum / 0.9375;
}

function ramp(x: number, lo: number, hi: number): number {
  const t = Math.min(1, Math.max(0, (x - lo) / (hi - lo)));
  return smooth(t);
}

/** Top edge of the hill as a fraction of the height, at fraction nx of the width. */
export function hillTop(nx: number): number {
  const bump = 0.5 + 0.5 * Math.cos(2 * Math.PI * (nx - 0.38));
  return 0.84 - 0.16 * bump;
}

function makeBank(seed: number, cx: number, size: number, base: number): Bank {
  const puffs: Puff[] = [];
  const n = 9;
  let hw = 0;
  let hh = 0;
  for (let i = 0; i < n; i++) {
    const k = i / (n - 1) - 0.5; // -0.5 .. 0.5 across the bank
    // Big in the middle, small at the ends: a heap with a flat underside.
    const r = size * (0.55 + 0.45 * (1 - Math.abs(k) * 2)) * (0.75 + 0.5 * hash(seed, i));
    const dx = k * size * 5.2 + (hash(seed + 7, i) - 0.5) * size;
    // Every circle rests on the base line, so the bottom is flat.
    puffs.push({ dx, cy: base - r, r });
    hw = Math.max(hw, Math.abs(dx) + r);
    hh = Math.max(hh, 2 * r);
  }
  return { cx, base, hw, hh, puffs };
}

const LAYERS: readonly Layer[] = [
  // Far: small, high, slow.
  {
    laps: 1,
    banks: [0, 1, 2, 3, 4].map((i) => makeBank(i + 1, (i + hash(i, 40)) / 5, 0.07, 0.24 + 0.1 * hash(i, 41))),
  },
  // Near: big heaps, lower, faster.
  {
    laps: 2,
    banks: [0, 1, 2].map((i) => makeBank(i + 20, (i + 0.2 + 0.6 * hash(i, 42)) / 3, 0.13, 0.55 + 0.1 * hash(i, 43))),
  },
];

/** Cumulus coverage at one point: 0 outside every puff, up to 1 deep inside one. */
function puffField(nx: number, ny: number, aspect: number, t: number): { d: number; shade: number } {
  let best = 0;
  let shade = 0;
  for (const layer of LAYERS) {
    const shift = (layer.laps * t) / LOOP_SECONDS;
    for (const bank of layer.banks) {
      let wx = nx - bank.cx - shift;
      wx -= Math.round(wx); // nearest copy: the bank wraps around the screen
      const x = wx * aspect;
      if (Math.abs(x) > bank.hw * 1.1 || ny > bank.base + 0.01 || ny < bank.base - bank.hh * 1.1) continue;
      for (const puff of bank.puffs) {
        const dx = x - puff.dx;
        const dy = ny - puff.cy;
        // A slow wobble of the rim makes the edge lumpy rather than a perfect circle.
        const r = puff.r * (0.92 + 0.16 * cloudDensity(puff.dx * 40 + (3 * dx) / puff.r, puff.cy * 40 + (3 * dy) / puff.r));
        const dist = Math.sqrt(dx * dx + dy * dy) / r;
        if (dist >= 1) continue;
        const depth = 1 - dist;
        if (depth > best) {
          best = depth;
          // Lit from the upper left; the underside and the right-hand side sit in shade.
          shade = Math.min(1, Math.max(0, 0.5 + 0.5 * (dy / r) + 0.3 * (dx / r)));
        }
      }
    }
  }
  return { d: best, shade };
}

/**
 * What one grid cell shows. `nx`,`ny` are the cell centre as fractions of the
 * canvas; `aspect` is width / height; `t` is seconds of drift.
 */
export function toneAt(nx: number, ny: number, aspect: number, t: number): Tone {
  const top = hillTop(nx);
  if (ny >= top) {
    // Lit from the upper left: the crest is bright, the foot darker.
    const depth = Math.min(1, (ny - top) / 0.2);
    const lit = 0.5 + 0.25 * (1 - depth) + 0.1 * (1 - nx);
    return { kind: "hill", b: Math.min(1, lit) };
  }

  const { d, shade } = puffField(nx, ny, aspect, t);
  if (d > 0) {
    // Dense bright fill with a rim that thins out, so the outline reads as round.
    const rim = ramp(d, 0, 0.35);
    return { kind: "cloud", b: Math.min(1, rim * (1 - 0.3 * shade) + 0.04) };
  }

  // Open sky: dark, sparse dots that thicken toward the horizon.
  return { kind: "sky", b: 0.08 + 0.3 * (ny / top) };
}

/** The glyph for a brightness. */
export function rampChar(b: number): string {
  const i = Math.round(Math.min(1, Math.max(0, b)) * (ASCII_RAMP.length - 1));
  return ASCII_RAMP[i]!;
}
