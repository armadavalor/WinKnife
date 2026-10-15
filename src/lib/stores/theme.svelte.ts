/**
 * Theme choice. Stored locally; applied by setting `data-theme` on <html>.
 * Graphite is the default and means "no attribute", so a fresh install renders
 * exactly as before this store existed.
 */
export const THEMES = ["graphite", "forest", "mist"] as const;
export type Theme = (typeof THEMES)[number];

const STORAGE_KEY = "winknife.theme";
const DEFAULT_THEME: Theme = "graphite";

export function isTheme(value: unknown): value is Theme {
  return typeof value === "string" && (THEMES as readonly string[]).includes(value);
}

function readStored(): Theme {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return isTheme(raw) ? raw : DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME;
  }
}

function applyToDocument(theme: Theme): void {
  if (typeof document === "undefined") return;
  if (theme === DEFAULT_THEME) delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}

class ThemeStore {
  current = $state<Theme>(DEFAULT_THEME);

  /** Call once from +layout.svelte on mount. */
  init(): void {
    this.current = readStored();
    applyToDocument(this.current);
  }

  set(theme: Theme): void {
    if (!isTheme(theme)) return;
    this.current = theme;
    applyToDocument(theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      // Storage blocked: the choice lasts for this session only.
    }
  }
}

export const theme = new ThemeStore();
