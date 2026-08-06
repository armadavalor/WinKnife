/**
 * Settings state.
 *
 * The one rule this store exists to enforce on the UI side: **what the page
 * shows after a save is what Rust stored, not what the page sent.** Retention
 * and the duplicate size floor are re-validated in Rust (a stored `0` used to
 * mean "purge everything on the next start"), so the response replaces the local
 * copy and the page says so when a value was corrected.
 */
import { getSettings, setSettings } from "$lib/api";
import { isApiError, type AppSettings } from "$lib/types";

/** Where the page is in its lifecycle. */
export type Status = "idle" | "loading" | "ready" | "saving" | "error";

function message(error: unknown): string {
  return isApiError(error) ? error.message : String(error);
}

/** Structural equality, used only to tell the user a value was corrected. */
function sameSettings(a: AppSettings, b: AppSettings): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

class SettingsStore {
  status = $state<Status>("idle");
  settings = $state<AppSettings | null>(null);
  error = $state<string | null>(null);
  /** True after a save that stored something other than what was sent. */
  adjusted = $state(false);
  /** True after a save that went through unchanged. */
  saved = $state(false);

  /** Load the settings. Safe to call repeatedly. */
  async load(): Promise<void> {
    if (this.status === "loading") {
      return;
    }
    this.status = "loading";
    this.error = null;
    try {
      this.settings = await getSettings();
      this.status = "ready";
    } catch (error: unknown) {
      this.error = message(error);
      this.status = "error";
    }
  }

  /** Turn one cleaner pack on or off locally; nothing is stored until save. */
  setPackEnabled(packId: string, enabled: boolean): void {
    const current = this.settings;
    if (!current) {
      return;
    }
    this.settings = {
      ...current,
      cleaner: {
        packs: current.cleaner.packs.map((pack) =>
          pack.id === packId ? { ...pack, enabled } : pack,
        ),
      },
    };
    this.saved = false;
  }

  /** Set the duplicate size floor locally. */
  setMinSizeBytes(bytes: number): void {
    const current = this.settings;
    if (!current) {
      return;
    }
    this.settings = {
      ...current,
      duplicates: { ...current.duplicates, minSizeBytes: bytes },
    };
    this.saved = false;
  }

  /** Set the retention window locally. */
  setRetentionDays(days: number): void {
    const current = this.settings;
    if (!current) {
      return;
    }
    this.settings = {
      ...current,
      quarantine: { ...current.quarantine, retentionDays: days },
    };
    this.saved = false;
  }

  /** Set the quarantine size cap locally. */
  setMaxBytes(bytes: number): void {
    const current = this.settings;
    if (!current) {
      return;
    }
    this.settings = {
      ...current,
      quarantine: { ...current.quarantine, maxBytes: bytes },
    };
    this.saved = false;
  }

  /** Save, then adopt whatever Rust says it actually stored. */
  async save(): Promise<void> {
    const sent = this.settings;
    if (!sent) {
      return;
    }
    this.status = "saving";
    this.error = null;
    this.saved = false;
    this.adjusted = false;
    try {
      const stored = await setSettings(sent);
      this.adjusted = !sameSettings(sent, stored);
      this.settings = stored;
      this.saved = true;
      this.status = "ready";
    } catch (error: unknown) {
      this.error = message(error);
      this.status = "ready";
    }
  }
}

/** Shared settings state. */
export const settingsStore = new SettingsStore();
