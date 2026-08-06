/**
 * Minimal translation helper.
 *
 * V1 ships English only, but every user-facing string goes through `t()` from
 * day one so adding Turkish is a dictionary drop-in rather than a refactor
 * (CONVENTIONS.md). Deliberately dependency-free: a `Record` lookup with
 * `{placeholder}` interpolation is all this app needs.
 */
import { APP_NAME, APP_TAGLINE } from "$lib/branding";

/** Locales the app knows about. */
export type Locale = "en";

/** Fallback locale, used whenever a key is missing from the active one. */
export const DEFAULT_LOCALE: Locale = "en";

const en = {
  "app.name": APP_NAME,
  "app.tagline": APP_TAGLINE,

  "nav.dashboard": "Dashboard",
  "nav.tools": "Tools",
  "nav.skipToContent": "Skip to content",

  "dashboard.title": "Dashboard",
  "dashboard.subtitle":
    "Every tool starts read-only. Nothing is removed without your say-so, and anything removed goes to quarantine first.",
  "dashboard.loading": "Loading tools…",
  "dashboard.empty": "No tools are registered yet.",
  "dashboard.error": "Could not load the tool registry.",
  "dashboard.retry": "Try again",

  "module.open": "Open",
  "module.capability.scan": "Scans",
  "module.capability.execute": "Applies changes",
  "module.capability.settings": "Has settings",
  "module.placeholder":
    "This tool has no features yet. Phase 0 only proves the wiring.",

  "action.cancel": "Cancel",
  "action.close": "Close",

  "risk.safe": "Safe to remove",
  "risk.needsReview": "Needs your review",
  "risk.doNotDelete": "Left alone",

  "cleaner.intro":
    "The scan is read-only: it looks at temporary files, Explorer's caches and your recycle bin, then shows you what it found before anything moves.",
  "cleaner.scan": "Scan for junk",
  "cleaner.scanAgain": "Scan again",
  "cleaner.scanning": "Looking around…",
  "cleaner.itemsScanned": "{count} items looked at",
  "cleaner.executing": "Moving files to quarantine…",
  "cleaner.summary":
    "About {safe} can be reclaimed safely, and {review} needs your review.",
  "cleaner.skippedRules":
    "Some rules could not run on this machine because their folders are not set: {rules}",
  "cleaner.safeHint": "selected by default",
  "cleaner.reviewHint": "pick these one by one",
  "cleaner.blockedHint": "shown for information, no action offered",
  "cleaner.bucketEmpty": "Nothing here.",
  "cleaner.otherGroup": "Other",
  "cleaner.selectGroup": "Select everything in {group}",
  "cleaner.selection": "{count} items selected · {size}",
  "cleaner.clean": "Clean up",
  "cleaner.confirmTitle": "Move {count} items to quarantine?",
  "cleaner.confirmBody":
    "This frees about {size}. Nothing is deleted — the files move to quarantine first.",
  "cleaner.confirmNote":
    "Everything goes to quarantine and can be restored for 7 days. After that it is purged automatically.",
  "cleaner.confirmRecycleBin":
    "Your selection includes emptying the recycle bin. That one is final and cannot be restored — the rest still can.",
  "cleaner.doneTitle": "Done",
  "cleaner.doneBody":
    "Reclaimed {size}. Everything moved is in quarantine if you want it back.",
  "cleaner.openQuarantine": "Open quarantine",

  "quarantine.title": "Quarantine",
  "quarantine.subtitle":
    "Everything the cleaner removed lives here until it is restored or the retention window runs out.",
  "quarantine.open": "Quarantine",
  "quarantine.held": "{count} items · {size} held",
  "quarantine.loading": "Loading…",
  "quarantine.empty": "Nothing is in quarantine.",
  "quarantine.batchLabel": "{count} items",
  "quarantine.keptFor": "Kept for another {time}",
  "quarantine.restore": "Restore",
  "quarantine.showItems": "Show items",
  "quarantine.hideItems": "Hide items",
  "quarantine.purgeNow": "Purge now",
  "quarantine.restored": "Restored {count} items.",
  "quarantine.restoredAlongside":
    "Something already occupied the original path, so this was restored as {path}.",

  "shutdown.noTimer": "No timer is set.",
  "shutdown.cancel": "Cancel timer",
  "shutdown.setTimer": "Set timer",
  "shutdown.actionLabel": "What should happen",
  "shutdown.actionShutdown": "Shut down",
  "shutdown.actionRestart": "Restart",
  "shutdown.actionSleep": "Sleep",
  "shutdown.presetsLabel": "Quick presets",
  "shutdown.preset30m": "In 30 minutes",
  "shutdown.preset1h": "In 1 hour",
  "shutdown.preset2h": "In 2 hours",
  "shutdown.presetTonight": "Tonight at 23:00",
  "shutdown.clockLabel": "Or at a specific time",
  "shutdown.willRunAt": "Will run at {time}",
  "shutdown.countdownFor.shutdown": "Shutting down in",
  "shutdown.countdownFor.restart": "Restarting in",
  "shutdown.countdownFor.sleep": "Going to sleep in",
  "shutdown.replaceTitle": "Replace the current timer?",
  "shutdown.replaceBody":
    "A timer is already running. Setting a new one replaces it.",
  "shutdown.trayHint":
    "WinKnife keeps the countdown itself, so it has to stay running. The tray icon shows the time left and can cancel the timer.",
  "shutdown.dryRunBanner":
    "This build was started with --dry-run: the timer counts down and warns you, but nothing is actually shut down. The action is written to the log instead.",

  "status.phase": "Phase 1 — first blood",
} as const;

/** Every key the dictionary knows. */
export type TranslationKey = keyof typeof en;

const dictionaries: Record<Locale, Partial<Record<string, string>>> = { en };

let activeLocale: Locale = DEFAULT_LOCALE;

/** The locale currently in use. */
export function getLocale(): Locale {
  return activeLocale;
}

/** Switch locale. Unknown keys fall back to {@link DEFAULT_LOCALE}. */
export function setLocale(locale: Locale): void {
  activeLocale = locale;
}

function interpolate(
  template: string,
  vars?: Readonly<Record<string, string | number>>,
): string {
  if (!vars) {
    return template;
  }
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = vars[name];
    return value === undefined ? match : String(value);
  });
}

function lookup(key: string): string | undefined {
  return dictionaries[activeLocale]?.[key] ?? dictionaries[DEFAULT_LOCALE]?.[key];
}

/**
 * Translate a known key.
 *
 * Returns the key itself if it is somehow missing, which makes gaps obvious in
 * the UI instead of rendering an empty string.
 */
export function t(
  key: TranslationKey,
  vars?: Readonly<Record<string, string | number>>,
): string {
  return interpolate(lookup(key) ?? key, vars);
}

/**
 * Translate a key that is only known at runtime, with a fallback.
 *
 * Module display names come from Rust manifests, so they cannot be typed keys.
 * The UI looks up `module.<id>.name` first and falls back to the English text
 * the manifest supplied — a Turkish dictionary then overrides them without any
 * Rust change.
 */
export function tDynamic(
  key: string,
  fallback: string,
  vars?: Readonly<Record<string, string | number>>,
): string {
  return interpolate(lookup(key) ?? fallback, vars);
}

/** Translated display name for a module manifest. */
export function tModuleName(id: string, fallback: string): string {
  return tDynamic(`module.${id}.name`, fallback);
}

/** Translated description for a module manifest. */
export function tModuleDescription(id: string, fallback: string): string {
  return tDynamic(`module.${id}.description`, fallback);
}
