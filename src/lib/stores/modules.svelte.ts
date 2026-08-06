/**
 * The module registry, as the UI sees it.
 *
 * Loaded once from `list_modules` and shared by the sidebar and the dashboard,
 * so the registry is fetched a single time per app start rather than per route.
 */
import { listModules } from "$lib/api";
import { isApiError, type ModuleManifest } from "$lib/types";

/** Lifecycle of the registry fetch. */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

class ModuleStore {
  /** Manifests in registry order; that order is the sidebar order. */
  modules = $state<ModuleManifest[]>([]);
  /** Where the fetch is. */
  status = $state<LoadStatus>("idle");
  /** Message of the last failure, if any. */
  error = $state<string | null>(null);

  /** Fetch the registry unless it is already loaded or in flight. */
  async load(): Promise<void> {
    if (this.status === "loading" || this.status === "ready") {
      return;
    }
    await this.reload();
  }

  /** Fetch the registry again, whatever the current state. */
  async reload(): Promise<void> {
    this.status = "loading";
    this.error = null;
    try {
      this.modules = await listModules();
      this.status = "ready";
    } catch (error: unknown) {
      this.modules = [];
      this.error = isApiError(error) ? error.message : String(error);
      this.status = "error";
    }
  }

  /** Look a manifest up by id. */
  find(id: string): ModuleManifest | undefined {
    return this.modules.find((module) => module.id === id);
  }
}

/** Shared registry state. */
export const moduleStore = new ModuleStore();
