/**
 * Background light state. The variant is stored locally (like the theme) and
 * set on <html> as `data-glow`. While animations run, the pointer position is
 * written to `--glow-x` / `--glow-y` once per frame so the light leans toward
 * the mouse; with animations off the light stays centred and still.
 */
import { glowPointer, parseGlowVariant, type GlowVariant } from "$lib/glow";
import { motion } from "$lib/stores/motion.svelte";

const STORAGE_KEY = "winknife.glow";
const CENTER = "50%";

function readStored(): GlowVariant {
  try {
    return parseGlowVariant(localStorage.getItem(STORAGE_KEY));
  } catch {
    return parseGlowVariant(null);
  }
}

function setPointer(x: string, y: string): void {
  const style = document.documentElement.style;
  style.setProperty("--glow-x", x);
  style.setProperty("--glow-y", y);
}

class GlowStore {
  variant = $state<GlowVariant>("aurora");

  private frame = 0;
  private pending: PointerEvent | null = null;

  /** Call once from +layout.svelte on mount, after `motion.init()`. */
  init(): void {
    if (typeof document === "undefined") return;
    this.variant = readStored();
    document.documentElement.dataset.glow = this.variant;
    setPointer(CENTER, CENTER);
    window.addEventListener("pointermove", (event) => this.onPointer(event), { passive: true });
  }

  set(variant: GlowVariant): void {
    this.variant = variant;
    document.documentElement.dataset.glow = variant;
    try {
      localStorage.setItem(STORAGE_KEY, variant);
    } catch {
      // Storage blocked: the choice lasts for this session only.
    }
  }

  private onPointer(event: PointerEvent): void {
    if (!motion.enabled) return;
    this.pending = event;
    if (this.frame !== 0) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      const last = this.pending;
      if (!last || !motion.enabled) return;
      const { x, y } = glowPointer(last.clientX, last.clientY, innerWidth, innerHeight);
      setPointer(`${x}%`, `${y}%`);
    });
  }
}

export const glow = new GlowStore();
