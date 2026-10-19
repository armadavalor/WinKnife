/**
 * Animations switch state. The toggle is stored locally (like the theme); the
 * system's reduced-motion setting is watched live. Whenever either changes,
 * `data-animations="off"` is set on <html> or removed.
 */
import { animationsEnabled, parseAnimationPref, type AnimationPref } from "$lib/motion";

const STORAGE_KEY = "winknife.animations";
const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

function readStored(): AnimationPref {
  try {
    return parseAnimationPref(localStorage.getItem(STORAGE_KEY));
  } catch {
    return parseAnimationPref(null);
  }
}

function applyToDocument(enabled: boolean): void {
  if (typeof document === "undefined") return;
  if (enabled) delete document.documentElement.dataset.animations;
  else document.documentElement.dataset.animations = "off";
}

class MotionStore {
  pref = $state<AnimationPref>("on");
  /** True while the system asks for reduced motion. */
  reducedMotion = $state(false);
  enabled = $derived(animationsEnabled(this.pref, this.reducedMotion));

  /** Call once from +layout.svelte on mount. */
  init(): void {
    this.pref = readStored();
    if (typeof matchMedia === "function") {
      const query = matchMedia(REDUCED_MOTION_QUERY);
      this.reducedMotion = query.matches;
      query.addEventListener("change", (event) => {
        this.reducedMotion = event.matches;
        applyToDocument(this.enabled);
      });
    }
    applyToDocument(this.enabled);
  }

  set(pref: AnimationPref): void {
    this.pref = pref;
    applyToDocument(this.enabled);
    try {
      localStorage.setItem(STORAGE_KEY, pref);
    } catch {
      // Storage blocked: the choice lasts for this session only.
    }
  }
}

export const motion = new MotionStore();
