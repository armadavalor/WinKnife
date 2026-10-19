/**
 * Background light (Polish item 7): a moving accent light behind every screen,
 * in a few variants to pick from, that leans toward the mouse.
 *
 * The variant lands on <html> as `data-glow`; the pointer lands as `--glow-x`
 * and `--glow-y` (percent of the window). The motion itself is CSS, so the
 * animations switch (`data-animations="off"`) stills it like everything else.
 */
export const GLOW_VARIANTS = ["aurora", "waves", "spotlight"] as const;
export type GlowVariant = (typeof GLOW_VARIANTS)[number];

const DEFAULT_VARIANT: GlowVariant = "aurora";
const CENTER = 50;
const FULL = 100;

export function isGlowVariant(value: unknown): value is GlowVariant {
  return typeof value === "string" && (GLOW_VARIANTS as readonly string[]).includes(value);
}

/** A stored value, read back; anything unrecognised means the default. */
export function parseGlowVariant(raw: unknown): GlowVariant {
  return isGlowVariant(raw) ? raw : DEFAULT_VARIANT;
}

function percent(position: number, size: number): number {
  if (size <= 0) return CENTER;
  return Math.round(Math.min(FULL, Math.max(0, (position / size) * FULL)));
}

/** A pointer position as whole percentages of the window, clamped to 0–100. */
export function glowPointer(
  clientX: number,
  clientY: number,
  width: number,
  height: number,
): { x: number; y: number } {
  return { x: percent(clientX, width), y: percent(clientY, height) };
}
