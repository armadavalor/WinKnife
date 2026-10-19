/**
 * The animations switch (Polish item 8): on by default, one toggle turns it
 * off, and the system's reduced-motion setting turns it off regardless.
 *
 * The effective answer lands on <html> as `data-animations="off"`; CSS and any
 * animated component read that one attribute, so a new animation honours the
 * switch by keying off it rather than re-deriving the rule.
 */
export const ANIMATION_PREFS = ["on", "off"] as const;
export type AnimationPref = (typeof ANIMATION_PREFS)[number];

const DEFAULT_PREF: AnimationPref = "on";

export function isAnimationPref(value: unknown): value is AnimationPref {
  return typeof value === "string" && (ANIMATION_PREFS as readonly string[]).includes(value);
}

/** A stored value, read back; anything unrecognised means the default. */
export function parseAnimationPref(raw: unknown): AnimationPref {
  return isAnimationPref(raw) ? raw : DEFAULT_PREF;
}

/** Whether animations run: the toggle is on and the system allows motion. */
export function animationsEnabled(pref: AnimationPref, reducedMotion: boolean): boolean {
  return pref === "on" && !reducedMotion;
}
