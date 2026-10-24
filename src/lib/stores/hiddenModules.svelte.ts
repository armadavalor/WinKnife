/**
 * Which tools the sidebar leaves out (issue #8). Stored locally, like the
 * theme: it is a per-machine view preference, not module settings.
 */
import { parseHiddenModules, toggleHidden } from "$lib/hiddenModules";

const STORAGE_KEY = "winknife.hiddenModules";

class HiddenModulesStore {
  ids = $state<string[]>([]);

  /** Call once from +layout.svelte on mount. */
  init(): void {
    try {
      this.ids = parseHiddenModules(localStorage.getItem(STORAGE_KEY));
    } catch {
      this.ids = [];
    }
  }

  isHidden(id: string): boolean {
    return this.ids.includes(id);
  }

  toggle(id: string): void {
    this.ids = toggleHidden(this.ids, id);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(this.ids));
    } catch {
      // Storage blocked: the choice lasts for this session only.
    }
  }
}

export const hiddenModules = new HiddenModulesStore();
