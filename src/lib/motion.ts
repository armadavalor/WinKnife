/**
 * The animations preference (Polish item 8): `system` (default) follows the
 * OS reduced-motion setting, `on` runs animations whatever the OS says (some
 * machines report "reduce" only because Windows' own effects are off), `off`
 * never does.
 *
 * The effective answer lands on <html> as `data-animations="off"`; CSS and any
 * animated component read that one attribute, so a new animation honours the
 * switch by keying off it rather than re-deriving the rule.
 */
export const ANIMATION_PREFS = ["system", "on", "off"] as const;
export type AnimationPref = (typeof ANIMATION_PREFS)[number];

const DEFAULT_PREF: AnimationPref = "system";

export function isAnimationPref(value: unknown): value is AnimationPref {
  return typeof value === "string" && (ANIMATION_PREFS as readonly string[]).includes(value);
}

/** A stored value, read back; anything unrecognised means the default. */
export function parseAnimationPref(raw: unknown): AnimationPref {
  return isAnimationPref(raw) ? raw : DEFAULT_PREF;
}

/**
 * The two-value preference stored before `system` existed: its "on" meant
 * "follow the OS", so it becomes `system` and nobody's behaviour changes.
 */
export function migrateLegacyPref(raw: unknown): AnimationPref {
  return raw === "off" ? "off" : DEFAULT_PREF;
}

/** Whether animations run. */
export function animationsEnabled(pref: AnimationPref, reducedMotion: boolean): boolean {
  if (pref === "system") return !reducedMotion;
  return pref === "on";
}
