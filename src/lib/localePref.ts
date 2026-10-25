/**
 * Which language the UI starts in. Pure so it can be tested without a browser:
 * a valid stored choice wins, otherwise Turkish when the Windows UI language
 * (what WebView2 reports as `navigator.language`) is Turkish, else English.
 */
export const LOCALES = ["en", "tr"] as const;
export type Locale = (typeof LOCALES)[number];

export function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && (LOCALES as readonly string[]).includes(value);
}

export function resolveLocale(stored: unknown, navigatorLanguage: string | undefined): Locale {
  if (isLocale(stored)) return stored;
  return navigatorLanguage?.toLowerCase().startsWith("tr") ? "tr" : "en";
}
