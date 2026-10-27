/** Sidebar and panel translucency, applied as CSS variables on <html>. */
import { clampOpacity, parseOpacity, PANEL_DEFAULT, SIDEBAR_DEFAULT } from "$lib/panelOpacity";

const SIDEBAR_KEY = "winknife.sidebarOpacity";
const PANEL_KEY = "winknife.panelOpacity";

function apply(sidebar: number, panel: number): void {
  if (typeof document === "undefined") return;
  document.documentElement.style.setProperty("--sidebar-alpha", `${sidebar}%`);
  document.documentElement.style.setProperty("--panel-alpha", `${panel}%`);
}

function save(key: string, value: number): void {
  try {
    localStorage.setItem(key, String(value));
  } catch {
    // Storage blocked: the choice lasts for this session only.
  }
}

class PanelOpacityStore {
  sidebar = $state(SIDEBAR_DEFAULT);
  panel = $state(PANEL_DEFAULT);

  /** Call once from +layout.svelte on mount. */
  init(): void {
    try {
      this.sidebar = parseOpacity(localStorage.getItem(SIDEBAR_KEY), SIDEBAR_DEFAULT);
      this.panel = parseOpacity(localStorage.getItem(PANEL_KEY), PANEL_DEFAULT);
    } catch {
      // Defaults stay.
    }
    apply(this.sidebar, this.panel);
  }

  setSidebar(value: number): void {
    this.sidebar = clampOpacity(value, SIDEBAR_DEFAULT);
    apply(this.sidebar, this.panel);
    save(SIDEBAR_KEY, this.sidebar);
  }

  setPanel(value: number): void {
    this.panel = clampOpacity(value, PANEL_DEFAULT);
    apply(this.sidebar, this.panel);
    save(PANEL_KEY, this.panel);
  }

  reset(): void {
    this.setSidebar(SIDEBAR_DEFAULT);
    this.setPanel(PANEL_DEFAULT);
  }
}

export const panelOpacity = new PanelOpacityStore();
