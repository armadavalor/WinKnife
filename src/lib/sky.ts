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

interface Layer {
  /** Lattice cells across the screen width. */
  scale: number;
  /** Lattice cells scrolled per second. */
  speed: number;
  /** Where the layer's pattern starts, so the two do not line up. */
  shift: number;
  /** How much of the layer's density shows. */
  weight: number;
}

const LAYERS: readonly Layer[] = [
  { scale: 2.6, speed: 0.045, shift: 3.1, weight: 0.8 },
  { scale: 4.2, speed: 0.09, shift: 7.7, weight: 1 },
];

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

  let cloud = 0;
  for (const layer of LAYERS) {
    const u = nx * layer.scale + layer.shift + t * layer.speed;
    // Clouds are wider than tall, so v spans fewer cells than u.
    const v = ny * layer.scale * 0.55 + layer.shift;
    const d = cloudDensity(u, v);
    cloud = Math.max(cloud, ramp(d, 0.5, 0.78) * layer.weight);
  }
  // Fade clouds into the haze above the hill so the horizon stays calm.
  cloud *= 1 - ramp(ny, top - 0.22, top - 0.02) * 0.8;
  if (cloud > 0.05) return { kind: "cloud", b: cloud };

  // Open sky: faint dots that thicken toward the horizon.
  return { kind: "sky", b: 0.1 + 0.3 * (ny / top) };
}

/** The glyph for a brightness. */
export function rampChar(b: number): string {
  const i = Math.round(Math.min(1, Math.max(0, b)) * (ASCII_RAMP.length - 1));
  return ASCII_RAMP[i]!;
}
