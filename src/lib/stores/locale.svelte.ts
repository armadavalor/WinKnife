/**
 * Interface language. Stored locally like the theme; `current` is reactive and
 * the layout re-renders its tree when it changes, so switching needs no restart.
 */
import { setLocale } from "$lib/i18n";
import { isLocale, resolveLocale, type Locale } from "$lib/localePref";

const STORAGE_KEY = "winknife.locale";

function readStored(): unknown {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function applyToDocument(locale: Locale): void {
  setLocale(locale);
  if (typeof document !== "undefined") document.documentElement.lang = locale;
}

class LocaleStore {
  current = $state<Locale>("en");

  /** Call once from +layout.svelte on mount. */
  init(): void {
    const nav = typeof navigator === "undefined" ? undefined : navigator.language;
    this.current = resolveLocale(readStored(), nav);
    applyToDocument(this.current);
  }

  set(locale: Locale): void {
    if (!isLocale(locale)) return;
    this.current = locale;
    applyToDocument(locale);
    try {
      localStorage.setItem(STORAGE_KEY, locale);
    } catch {
      // Storage blocked: the choice lasts for this session only.
    }
  }
}

export const locale = new LocaleStore();
