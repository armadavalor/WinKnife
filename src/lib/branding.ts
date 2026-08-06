/**
 * The one place the UI spells the product name.
 *
 * `cleanify` is a codename; the public name is still open (PLAN.md section 7).
 * Every user-visible occurrence derives from `APP_NAME`, so renaming the UI is
 * a one-line change. The other places the codename appears (Cargo.toml,
 * tauri.conf.json, package.json) are listed in the README rename checklist.
 */
export const APP_NAME = "Cleanify";

/** Shown under the app name in the sidebar. */
export const APP_TAGLINE = "Windows, tidied safely";
