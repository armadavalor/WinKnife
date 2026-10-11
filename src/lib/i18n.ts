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
  "dashboard.error": "Could not load what this machine looks like right now.",
  "dashboard.retry": "Try again",
  "dashboard.reclaimable": "Could be reclaimed",
  "dashboard.reclaimableNote":
    "Estimated from the last time each tool ran. Nothing has been removed.",
  "dashboard.quarantineNote": "{count} items held · open the drawer",
  "dashboard.timer": "Shutdown timer",
  "dashboard.timerActive": "Runs at {when}",
  "dashboard.free": "free",
  "dashboard.lastScan": "Last run {when} · {count} found",
  "dashboard.couldFree": "about {size} reclaimable",
  "dashboard.wasCancelled": "stopped early",
  "dashboard.neverRun": "Never run on this machine.",
  "dashboard.noScan": "Nothing to scan — this one works on its own.",
  "dashboard.review": "Review startup items",

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
  "cleaner.previousScan": "Previous scan",
  "cleaner.historyNotice": "This is a past estimate. Scan again to review current files before making changes.",
  "cleaner.cameFrom":
    "You came here from {path} on the disk map. The cleaner does not scan a folder you pick — it looks at the known junk locations its rule packs describe. If that folder is one of them it will show up below; if it is not, nothing here will touch it.",
  "cleaner.scanAgain": "Scan again",
  "cleaner.scanning": "Looking around…",
  "cleaner.itemsScanned": "{count} items looked at",
  "cleaner.executing": "Moving files to quarantine…",
  "cleaner.summary":
    "About {safe} can be reclaimed safely, and {review} needs your review.",
  "cleaner.skippedRules": "Some rules could not run on this machine: {rules}",
  "cleaner.collapseGroup": "Collapse {group}",
  "cleaner.expandGroup": "Expand {group}",
  "cleaner.groupCount": "{count} items",
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
    "Everything goes to quarantine first. You can put any of it back from the quarantine drawer for the next {days} days; after that it is purged automatically. Nothing is deleted right now.",
  "cleaner.confirmRecycleBin":
    "Your selection includes emptying the recycle bin. That one is final and cannot be restored — the rest still can.",
  "cleaner.doneTitle": "Done",
  "cleaner.doneBody":
    "Reclaimed {size}. Everything moved is in quarantine if you want it back.",
  "cleaner.openQuarantine": "Open quarantine",

  "nav.settings": "Settings",

  "duplicates.intro":
    "The scan reads your files and never writes to them. It compares sizes first, then the ends of each file, then the whole thing — so two files are only ever called duplicates when every byte matches.",
  "duplicates.scan": "Find duplicates",
  "duplicates.pinnedRoot": "Only looking inside {path}, because you came here from the disk map.",
  "duplicates.clearPinnedRoot": "Use my saved folders instead",
  "duplicates.scanAgain": "Scan again",
  "duplicates.scanning": "Comparing files…",
  "duplicates.stop": "Stop",
  "duplicates.stopping": "Stopping…",
  "duplicates.cancelled":
    "Stopped before finishing, so nothing is listed. Partial results would only be half an answer.",
  "duplicates.summary": "{size} wasted in {sets} duplicate sets",
  "duplicates.empty": "No duplicates found in the folders that were scanned.",
  "duplicates.hashed": "{count} files read in full",
  "duplicates.setLabel": "{count} copies · {size} each",
  "duplicates.wasted": "{size} wasted",
  "duplicates.keeper": "Suggested keeper",
  "duplicates.makeKeeper": "Keep this one instead",
  "duplicates.keepNewest": "Keep the newest",
  "duplicates.keepOldest": "Keep the oldest",
  "duplicates.selectAll": "Select every non-keeper",
  "duplicates.selectNone": "Clear the selection",
  "duplicates.filterClass": "File type",
  "duplicates.filterMinSize": "Smallest set to show",
  "duplicates.classAll": "Everything",
  "duplicates.classImages": "Images",
  "duplicates.classVideo": "Video",
  "duplicates.classDocuments": "Documents",
  "duplicates.classOther": "Other",
  "duplicates.linkedTitle": "Excluded: same file under another name",
  "duplicates.skippedRoots": "Some folders were not scanned: {roots}",
  "duplicates.selection": "{count} copies selected · {size}",
  "duplicates.remove": "Move copies to quarantine",
  "duplicates.confirmTitle": "Move {count} copies to quarantine?",
  "duplicates.confirmBody":
    "This frees about {size}. The suggested keeper of every set stays where it is.",
  "duplicates.doneTitle": "Done",
  "duplicates.doneBody":
    "Reclaimed {size}. Everything moved is in quarantine if you want it back.",

  "treemap.title": "Disk map",
  "treemap.intro":
    "This walks the drive read-only and adds up every folder, so the rectangles are sizes rather than guesses. Click a folder to go in, use the breadcrumb to come back out, right-click for what to do about it.",
  "treemap.drive": "Drive",
  "treemap.loadingDrives": "Looking for drives…",
  "treemap.drivesError": "Could not list this machine's drives.",
  "treemap.noDrives": "No drives were found that can be read.",
  "treemap.scan": "Map this drive",
  "treemap.rescan": "Map it again",
  "treemap.stop": "Stop",
  "treemap.scanning": "{count} items measured so far…",
  "treemap.opening": "Opening…",
  "treemap.capacity": "{used} used of {total} · {free} free",
  "treemap.capacityLabel": "Space used on this drive",
  "treemap.staleness": "Measured {when} · {size} across {files} files",
  "treemap.partial": "stopped early, so this is part of the picture",
  "treemap.unreadable":
    "{count} folders could not be read, so some space is unaccounted for",
  "treemap.breadcrumb": "Folder path",
  "treemap.canvasLabel":
    "Disk map. Arrow keys move between folders, Enter opens one, Backspace goes up.",
  "treemap.empty": "This drive has not been mapped yet.",
  "treemap.folderEmpty": "There is nothing in this folder.",
  "treemap.folderUnreadable":
    "This folder could not be opened, so what is inside it is unknown.",
  "treemap.sizeUnknown": "size unknown",
  "treemap.sizeUnknownLong":
    "Size unknown — part of this folder could not be read, so any figure would be a guess.",
  "treemap.linkNote":
    "A junction. Its contents live elsewhere on this drive and are counted there, not here.",
  "treemap.foldedNote": "{count} items grouped together",
  "treemap.compressedNote":
    "{count} files below here are compressed or sparse, so they take up less room than shown.",
  "treemap.linksNote":
    "{count} junctions were drawn but not counted, so nothing is measured twice.",
  "treemap.openInExplorer": "Open in Explorer",
  "treemap.scanWithCleaner": "Scan with Cleaner",
  "treemap.findDuplicates": "Find duplicates here",
  "treemap.category.directory": "Folder",
  "treemap.category.video": "Video",
  "treemap.category.image": "Images",
  "treemap.category.audio": "Audio",
  "treemap.category.archive": "Archives",
  "treemap.category.code": "Code",
  "treemap.category.app": "Programs",
  "treemap.category.document": "Documents",
  "treemap.category.other": "Other",

  "startup.intro":
    "Reading this list changes nothing. Turning an entry off uses the same switch Windows' own Task Manager uses, so the two agree — nothing is deleted, and you can turn it back on here whenever you like.",
  "startup.loading": "Reading your startup entries…",
  "startup.loadError": "Could not read what runs at logon.",
  "startup.empty": "Nothing runs at logon on this machine.",
  "startup.summary": "{total} entries · {on} currently run at logon",
  "startup.refresh": "Refresh",
  "startup.unreadable":
    "Some locations could not be read, so this list may be incomplete:",
  "startup.signed": "signed",
  "startup.unsigned": "unsigned",
  "startup.impact": "{tier} impact (estimated)",
  "startup.impactLow": "Low",
  "startup.impactMedium": "Medium",
  "startup.impactHigh": "High",
  "startup.verdictKeep": "Best left on",
  "startup.verdictUserChoice": "Your call",
  "startup.verdictSafeToDisable": "Safe to turn off",
  "startup.verdictUnknown": "Not recognised",
  "startup.on": "On",
  "startup.off": "Off",
  "startup.working": "Working…",
  "startup.toggleLabel": "Run {name} at logon",
  "startup.confirmTitle": "Stop {name} running at logon?",
  "startup.confirmBody":
    "It will not start the next time you log in. You can still open it yourself whenever you want to.",
  "startup.confirmNote":
    "Nothing is deleted. A registry entry keeps its value and gets the same disabled flag Task Manager uses; a shortcut is moved into WinKnife's own folder and moved back untouched if you turn this on again.",
  "startup.confirmAction": "Turn it off",

  "settings.title": "Settings",
  "settings.subtitle":
    "Everything here is stored locally. Values that could make the app unsafe are corrected on save, and the page shows you what was actually stored.",
  "settings.loading": "Loading settings…",
  "settings.error": "Could not load your settings.",
  "settings.save": "Save settings",
  "settings.saving": "Saving…",
  "settings.saved": "Saved.",
  "settings.adjusted":
    "Some values were outside the allowed range and have been corrected.",
  "settings.packsTitle": "Cleaner rule packs",
  "settings.packsSubtitle":
    "A pack that is off is not scanned at all — not scanned and hidden, but never walked.",
  "settings.packDefaultOn": "on by default",
  "settings.packDefaultOff": "off by default",
  "settings.packRules": "{count} rules",
  "settings.duplicatesTitle": "Duplicate finder",
  "settings.minSize": "Ignore files smaller than",
  "settings.minSizeHint":
    "Below {size} the scan finds noise rather than duplicates, so that is the floor.",
  "settings.quarantineTitle": "Quarantine",
  "settings.retentionDays": "Keep removed files for",
  "settings.retentionHint":
    "Between {min} and {max} days. A retention of zero would make everything purgeable on the next start, so it is not allowed.",
  "settings.maxBytes": "Stop the quarantine growing past",
  "settings.days": "days",

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
  "quarantine.unreadableRows":
    "{count} rows of the ledger can no longer be read, so they are not included in the totals above. The files they refer to are still in the quarantine folder and have not been touched.",
  "quarantine.purged": "Purged {count} items and freed {size}.",
  "quarantine.affectedBatches":
    "That came out of {count} batches — the size cap removes the oldest items first, so a batch can lose part of itself and still be listed.",
  "quarantine.purgeFailed":
    "{count} items could not be removed and will be retried on the next purge.",
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

  "status.phase": "Phase 3 — disk map + startup",
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
