/**
 * How see-through the frosted panels are, as a percentage of paint kept.
 * The floor keeps text readable over the busiest backdrop.
 */
export const OPACITY_MIN = 30;
export const OPACITY_MAX = 100;
export const SIDEBAR_DEFAULT = 70;
export const PANEL_DEFAULT = 60;

export function clampOpacity(value: number, fallback: number): number {
  if (!Number.isFinite(value)) return fallback;
  return Math.min(OPACITY_MAX, Math.max(OPACITY_MIN, Math.round(value)));
}

export function parseOpacity(raw: string | null, fallback: number): number {
  return raw === null ? fallback : clampOpacity(Number(raw), fallback);
}
