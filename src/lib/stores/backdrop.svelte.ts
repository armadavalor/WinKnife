/** Which background is drawn behind the app. Stored locally, like the theme. */
import { BACKDROPS, isBackdrop, type Backdrop } from "$lib/sky";

const STORAGE_KEY = "winknife.backdrop";
const DEFAULT: Backdrop = "halftone";

function readStored(): Backdrop {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return isBackdrop(raw) ? raw : DEFAULT;
  } catch {
    return DEFAULT;
  }
}

class BackdropStore {
  value = $state<Backdrop>(DEFAULT);

  /** Call once from +layout.svelte on mount. */
  init(): void {
    this.value = readStored();
  }

  set(value: Backdrop): void {
    if (!BACKDROPS.includes(value)) return;
    this.value = value;
    try {
      localStorage.setItem(STORAGE_KEY, value);
    } catch {
      // Storage blocked: the choice lasts for this session only.
    }
  }
}

export const backdrop = new BackdropStore();
