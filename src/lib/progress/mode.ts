/**
 * How a progress bar should look. Kept out of the component so the rule is
 * testable without a DOM (see mode.test.ts).
 */

/** Unknown total, known total still moving, or a still bar. */
export type ProgressMode = "indeterminate" | "running" | "static";

const PCT_MIN = 0;
const PCT_MAX = 100;

export function clampPct(pct: number | null): number | null {
  if (pct === null) return null;
  if (Number.isNaN(pct)) return PCT_MIN;
  return Math.min(PCT_MAX, Math.max(PCT_MIN, pct));
}

/**
 * `active` is false for gauges that are not work in progress (disk capacity);
 * those never move. Motion itself is switched off in CSS under
 * `prefers-reduced-motion` and `data-animations="off"`.
 */
export function progressMode(pct: number | null, active: boolean): ProgressMode {
  const clamped = clampPct(pct);
  if (clamped === null) return "indeterminate";
  if (!active || clamped >= PCT_MAX) return "static";
  return "running";
}
