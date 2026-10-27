/** Which tools the dashboard shows as tiles. Stored locally, like hiddenModules. */
import { parsePinnedModules, togglePinned } from "$lib/pinnedModules";

const STORAGE_KEY = "winknife.pinnedModules";

class PinnedModulesStore {
  ids = $state<string[]>(parsePinnedModules(null));

  /** Call once from +layout.svelte on mount. */
  init(): void {
    try {
      this.ids = parsePinnedModules(localStorage.getItem(STORAGE_KEY));
    } catch {
      this.ids = parsePinnedModules(null);
    }
  }

  toggle(id: string): void {
    this.ids = togglePinned(this.ids, id);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(this.ids));
    } catch {
      // Storage blocked: the choice lasts for this session only.
    }
  }
}

export const pinnedModulesStore = new PinnedModulesStore();
