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
export type Locale = "en" | "tr";

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
  "startup.offLeftover": "Off — but its shortcut is still in the Startup folder",
  "startup.leftoverReason":
    "WinKnife holds this shortcut as switched off, but a shortcut with the same name is back at {path}, so it still runs at logon. Turning it on keeps that shortcut and moves WinKnife's copy to the quarantine if the two are identical; if they differ, nothing is changed and you decide which to keep.",
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
  "settings.maxBytesHint":
    "Encrypted items kept after a refused restore are not counted against this limit; only you can remove them.",
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
  "quarantine.restoreRefused": "restore refused: encrypted; kept until you remove it",
  "quarantine.removeItem": "Remove",
  "quarantine.removeTitle": "Remove this item for good?",
  "quarantine.removeBody":
    "{path} could not be restored because it is encrypted, so the automatic purge keeps it.",
  "quarantine.removeWarning": "Removing it deletes it permanently. This cannot be undone.",
  "quarantine.removeNote":
    "To keep it instead, decrypt it where it is held (the path below: Properties, Advanced, clear “Encrypt contents”) and restore it again.",
  "quarantine.heldAt": "Held at",
  "quarantine.removeFailed":
    "{path} could not be removed: its file could not be deleted. It is still held, and the automatic purge never removes it; try again, or decrypt it there and restore it.",
  "quarantine.removeRefused":
    "{path} was not removed: it is no longer held here. Refresh the list to see where it stands.",
  "quarantine.removeNotMarked":
    "{path} was not removed: its restore was never refused, so it is not an item you remove by hand. It stays held until its retention window ends, and you can still restore it.",
  "quarantine.removeError": "{path} could not be removed: {error}.",
  "quarantine.refusedHeld":
    "Encrypted items kept after a refused restore: {count}, using {used}. They are not counted against the {cap} size limit, and nothing removes them but you: remove them here, or decrypt them and restore them.",
  "quarantine.removeAction": "Remove permanently",
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

  "restorePoint.statusLabel": "System Protection",
  "restorePoint.statusLoading": "Reading System Protection…",
  "restorePoint.protection.on": "Looks on",
  "restorePoint.protection.off": "Looks off",
  "restorePoint.protection.unknown": "Unknown",
  "restorePoint.descriptionLabel": "Description (optional)",
  "restorePoint.descriptionHint":
    "Up to {max} characters. Left empty, the default name is used.",
  "restorePoint.uacNotice":
    "Creating a restore point needs administrator rights, so Windows will ask for permission (UAC) after you click. WinKnife does not change your System Protection settings.",
  "restorePoint.create": "Create restore point",
  "restorePoint.creating": "Waiting for Windows…",
  "restorePoint.outcome.created": "Restore point created.",
  "restorePoint.outcome.skipped":
    "Windows skipped it: it allows only one restore point every 24 hours, and one was made recently.",
  "restorePoint.outcome.maybeSkipped":
    "Windows accepted the request but WinKnife could not confirm a new restore point. Windows may have skipped it (24-hour limit).",
  "restorePoint.outcome.protectionOff":
    "System Protection is off, so Windows cannot create restore points.",
  "restorePoint.outcome.declined":
    "No restore point was created: the administrator prompt was declined.",
  "restorePoint.outcome.timedOut":
    "Windows did not finish within 5 minutes. It may still be working; check again later.",
  "restorePoint.outcome.failed": "Creating the restore point failed (error code {code}).",
  "restorePoint.settingsHint":
    "You can turn System Protection on in Windows' own settings. WinKnife only opens the dialog.",
  "restorePoint.openSettings": "Open System Protection settings",
  "startup.restorePointLink": "Create a restore point first",
  // Same text as the Rust manifest; listed so Turkish can override it.
  "module.restore-point.name": "Restore Point",
  "module.restore-point.description":
    "Create a Windows restore point before you change system settings.",
  "module.empty-folders.name": "Empty Folders",
  "module.empty-folders.description":
    "Find folders that contain no files at any depth, and move them to quarantine.",

  "keepAwake.presetsLabel": "For how long",
  "keepAwake.preset30m": "30 minutes",
  "keepAwake.preset1h": "1 hour",
  "keepAwake.preset2h": "2 hours",
  "keepAwake.preset4h": "4 hours",
  "keepAwake.customLabel": "Or a custom number of minutes (1 to 1440)",
  "keepAwake.start": "Keep awake",
  "keepAwake.stop": "Stop",
  "keepAwake.displayLabel": "Keep the screen on too",
  "keepAwake.idle": "The PC may sleep as usual.",
  "keepAwake.activeFor": "Staying awake for",
  "keepAwake.until": "Until {time}",
  "keepAwake.activeDisplay": "The screen stays on as well.",
  "keepAwake.noPersist":
    "Nothing is saved. Closing WinKnife, or restarting the PC, ends keep-awake; the longest session is 24 hours.",
  "keepAwake.shutdownArmed":
    "A shutdown timer is armed. Keep-awake does not cancel it: the PC will still shut down, restart or sleep when that timer ends.",
  "keepAwake.invalidMinutes": "Enter a whole number from 1 to 1440.",

  "emptyFolders.intro":
    "Finds folders that hold no files at any depth. Nothing starts selected, and anything you remove goes to quarantine, where you can restore it.",
  "emptyFolders.rootLabel": "Folder to scan",
  "emptyFolders.rootHint": "Leave empty to scan your user profile.",
  "emptyFolders.scan": "Find empty folders",
  "emptyFolders.scanning": "Looking for empty folders…",
  "emptyFolders.stop": "Stop",
  "emptyFolders.stopping": "Stopping…",
  "emptyFolders.scanAgain": "Scan again",
  "emptyFolders.summary": "{count} empty folders found in {visited} folders.",
  "emptyFolders.cancelled":
    "The scan was stopped early, so this list is incomplete.",
  "emptyFolders.unreadable":
    "{count} folders could not be read and were left alone.",
  "emptyFolders.skippedProtected":
    "{count} protected locations were not entered.",
  "emptyFolders.nested": "includes {count} empty folders inside",
  "emptyFolders.empty": "No empty folders found.",
  "emptyFolders.selectAll": "Select all",
  "emptyFolders.selectNone": "Select none",
  "emptyFolders.selection": "{count} folders selected",
  "emptyFolders.remove": "Move folders to quarantine",
  "emptyFolders.confirmTitle": "Move {count} empty folders to quarantine?",
  "emptyFolders.confirmBody":
    "Each folder is checked again first; one that is no longer empty is skipped.",
  "emptyFolders.doneTitle": "Done",
  "emptyFolders.doneBody": "{count} folders moved to quarantine.",
  "unlocker.title": "File Unlocker",
  "unlocker.subtitle":
    "Find out which program is holding a file open. Checking is read-only; a program is only ever asked to close, never forced.",
  "unlocker.pickFiles": "Choose files…",
  "unlocker.pickFolder": "Choose a folder…",
  "unlocker.dropHint": "Or drop files and folders anywhere on this window.",
  "unlocker.dropActive": "Drop to check who holds these",
  "unlocker.pathLabel": "Path",
  "unlocker.pathPlaceholder": "C:\\path\\to\\file.docx",
  "unlocker.addPath": "Add",
  "unlocker.pickError": "The file picker could not open. Drop the files here or type a path instead.",
  "unlocker.picked": "Checking {count} picked paths",
  "unlocker.clear": "Clear",
  "unlocker.recheck": "Check again",
  "unlocker.checking": "Asking Windows who holds these files…",
  "unlocker.queryError": "Windows could not answer the question.",
  "unlocker.filesChecked": "{count} files checked",
  "unlocker.capNote":
    "This folder has more files than WinKnife checks at once: only the first {cap} files were checked.",
  "unlocker.unreadable": "{count} entries inside the folder could not be read and were not checked.",
  "unlocker.skippedTitle": "Not checked",
  "unlocker.emptyTitle": "Nothing is holding these files open.",
  "unlocker.emptyHint":
    "If the file still cannot be changed, it may be read-only, or your account may not have permission to change it.",
  "unlocker.pid": "PID {pid}",
  "unlocker.service": "service {name}",
  "unlocker.kind.window_app": "Window app",
  "unlocker.kind.console": "Console",
  "unlocker.kind.service": "Service",
  "unlocker.kind.explorer": "Explorer",
  "unlocker.kind.critical": "System-critical",
  "unlocker.kind.unknown": "Unknown",
  "unlocker.holdsSome": "Has {count} of these files open.",
  "unlocker.holdsUnattributed": "Holds one or more of these files.",
  "unlocker.heldMore": "and {count} more",
  "unlocker.reason.window":
    "Window app. It can be asked to close, the same way Windows asks when you sign out.",
  "unlocker.reason.console":
    "Console program. It can be asked to close; it may stop whatever it is running.",
  "unlocker.reason.service": "Windows service — stop it from Services if you are sure.",
  "unlocker.reason.explorer":
    "Explorer — restarting it is safer than closing it; WinKnife does not do this for you.",
  "unlocker.reason.critical":
    "Critical system process — closing it could crash or restart Windows, so WinKnife never asks.",
  "unlocker.reason.unknown":
    "Windows could not say what kind of program this is, so WinKnife does not offer to close it.",
  "unlocker.reason.system":
    "Windows itself (the System process) holds this, often because the file is shared over the network or being indexed. It cannot be closed.",
  "unlocker.reason.self": "This is WinKnife itself. Close the screen that is using the file instead.",
  "unlocker.askClose": "Ask to close",
  "unlocker.asking": "Asking…",
  "unlocker.openLocation": "Open file location",
  "unlocker.confirmTitle": "Ask {app} to close?",
  "unlocker.confirmMessage":
    "{app} will be asked to close. If it has unsaved work, it may ask you to save first.",
  "unlocker.confirmNote": "WinKnife never forces a program to close or ends it.",
  "unlocker.outcome.closed": "{app} closed.",
  "unlocker.outcome.still_running":
    "{app} is still running. It may be waiting for you to save, or it chose not to close.",
  "unlocker.outcome.already_exited": "{app} had already closed.",
  "unlocker.outcome.refused": "{app} was not asked to close: {reason}",
  "unlocker.closeError": "Could not ask {app} to close.",

  "status.phase": "Phase 3 — disk map + startup",
} as const;

/** Turkish overrides. Missing keys fall back to English. */
const tr: Partial<Record<TranslationKey, string>> = {
  "quarantine.removeNotMarked":
    "{path} kaldırılmadı: geri yüklemesi hiç reddedilmedi, bu yüzden elle kaldırılan bir öğe değil. Saklama süresi bitene kadar karantinada kalır ve hâlâ geri yükleyebilirsiniz.",
  "restorePoint.statusLabel": "Sistem Koruması",
  "restorePoint.statusLoading": "Sistem Koruması okunuyor…",
  "restorePoint.protection.on": "Açık görünüyor",
  "restorePoint.protection.off": "Kapalı görünüyor",
  "restorePoint.protection.unknown": "Bilinmiyor",
  "restorePoint.descriptionLabel": "Açıklama (isteğe bağlı)",
  "restorePoint.descriptionHint":
    "En fazla {max} karakter. Boş bırakılırsa varsayılan ad kullanılır.",
  "restorePoint.uacNotice":
    "Geri yükleme noktası oluşturmak yönetici izni gerektirir; tıkladıktan sonra Windows izin isteyecek (UAC). WinKnife Sistem Koruması ayarlarınızı değiştirmez.",
  "restorePoint.create": "Geri yükleme noktası oluştur",
  "restorePoint.creating": "Windows bekleniyor…",
  "restorePoint.outcome.created": "Geri yükleme noktası oluşturuldu.",
  "restorePoint.outcome.skipped":
    "Windows atladı: 24 saatte yalnızca bir geri yükleme noktasına izin veriyor ve yakın zamanda bir tane oluşturulmuş.",
  "restorePoint.outcome.maybeSkipped":
    "Windows isteği kabul etti ama WinKnife yeni bir geri yükleme noktasını doğrulayamadı. Windows atlamış olabilir (24 saat sınırı).",
  "restorePoint.outcome.protectionOff":
    "Sistem Koruması kapalı, bu yüzden Windows geri yükleme noktası oluşturamıyor.",
  "restorePoint.outcome.declined":
    "Geri yükleme noktası oluşturulmadı: yönetici izni reddedildi.",
  "restorePoint.outcome.timedOut":
    "Windows 5 dakika içinde bitirmedi. Hâlâ çalışıyor olabilir; daha sonra yeniden kontrol edin.",
  "restorePoint.outcome.failed": "Geri yükleme noktası oluşturulamadı (hata kodu {code}).",
  "restorePoint.settingsHint":
    "Sistem Korumasını Windows'un kendi ayarlarından açabilirsiniz. WinKnife yalnızca pencereyi açar.",
  "restorePoint.openSettings": "Sistem Koruması ayarlarını aç",
  "startup.restorePointLink": "Önce bir geri yükleme noktası oluşturun",
  "module.restore-point.name": "Geri Yükleme Noktası",
  "module.restore-point.description":
    "Sistem ayarlarını değiştirmeden önce bir Windows geri yükleme noktası oluşturun.",
  "startup.offLeftover": "Kapalı — ama kısayolu hâlâ Başlangıç klasöründe",
  "startup.leftoverReason":
    "WinKnife bu kısayolu kapalı olarak tutuyor, ancak aynı adlı bir kısayol {path} konumuna geri gelmiş; bu yüzden oturum açılışında hâlâ çalışıyor. Açtığınızda o kısayol kalır; ikisi aynıysa WinKnife'ın kopyası karantinaya taşınır, farklıysa hiçbir şey değişmez ve hangisini tutacağınıza siz karar verirsiniz.",
  "module.empty-folders.name": "Boş klasörler",
  "module.empty-folders.description":
    "Hiçbir derinlikte dosya içermeyen klasörleri bulur ve karantinaya taşır.",
  "emptyFolders.intro":
    "Hiçbir derinlikte dosya içermeyen klasörleri bulur. Başlangıçta hiçbiri seçili değildir; kaldırdığınız her şey karantinaya gider ve geri alınabilir.",
  "emptyFolders.rootLabel": "Taranacak klasör",
  "emptyFolders.rootHint": "Boş bırakırsanız kullanıcı profiliniz taranır.",
  "emptyFolders.scan": "Boş klasörleri bul",
  "emptyFolders.scanning": "Boş klasörler aranıyor…",
  "emptyFolders.stop": "Durdur",
  "emptyFolders.stopping": "Durduruluyor…",
  "emptyFolders.scanAgain": "Yeniden tara",
  "emptyFolders.summary": "{visited} klasörde {count} boş klasör bulundu.",
  "emptyFolders.cancelled": "Tarama erken durduruldu, bu liste eksik.",
  "emptyFolders.unreadable": "{count} klasör okunamadı ve dokunulmadı.",
  "emptyFolders.skippedProtected": "{count} korumalı konuma girilmedi.",
  "emptyFolders.nested": "içinde {count} boş klasör daha var",
  "emptyFolders.empty": "Boş klasör bulunamadı.",
  "emptyFolders.selectAll": "Tümünü seç",
  "emptyFolders.selectNone": "Seçimi kaldır",
  "emptyFolders.selection": "{count} klasör seçili",
  "emptyFolders.remove": "Klasörleri karantinaya taşı",
  "emptyFolders.confirmTitle": "{count} boş klasör karantinaya taşınsın mı?",
  "emptyFolders.confirmBody":
    "Her klasör önce yeniden kontrol edilir; artık boş olmayan atlanır.",
  "emptyFolders.doneTitle": "Tamamlandı",
  "emptyFolders.doneBody": "{count} klasör karantinaya taşındı.",
  "keepAwake.presetsLabel": "Ne kadar süreyle",
  "keepAwake.preset30m": "30 dakika",
  "keepAwake.preset1h": "1 saat",
  "keepAwake.preset2h": "2 saat",
  "keepAwake.preset4h": "4 saat",
  "keepAwake.customLabel": "Veya özel dakika sayısı (1 ile 1440 arası)",
  "keepAwake.start": "Uyanık tut",
  "keepAwake.stop": "Durdur",
  "keepAwake.displayLabel": "Ekran da açık kalsın",
  "keepAwake.idle": "Bilgisayar her zamanki gibi uykuya geçebilir.",
  "keepAwake.activeFor": "Uyanık kalma süresi",
  "keepAwake.until": "Bitiş: {time}",
  "keepAwake.activeDisplay": "Ekran da açık kalır.",
  "keepAwake.noPersist":
    "Hiçbir şey kaydedilmez. WinKnife'ı kapatmak ya da bilgisayarı yeniden başlatmak uyanık tutmayı sonlandırır; en uzun oturum 24 saattir.",
  "keepAwake.shutdownArmed":
    "Bir kapatma zamanlayıcısı kurulu. Uyanık tutma onu iptal etmez: zamanlayıcı bitince bilgisayar yine kapanır, yeniden başlar ya da uykuya geçer.",
  "keepAwake.invalidMinutes": "1 ile 1440 arasında tam sayı girin.",
  "unlocker.title": "Dosya Kilidi Açıcı",
  "unlocker.subtitle":
    "Bir dosyayı hangi programın açık tuttuğunu bulun. Kontrol salt okunurdur; bir programdan yalnızca kapanması istenir, asla zorlanmaz.",
  "unlocker.pickFiles": "Dosya seç…",
  "unlocker.pickFolder": "Klasör seç…",
  "unlocker.dropHint": "Ya da dosya ve klasörleri bu pencerenin herhangi bir yerine bırakın.",
  "unlocker.dropActive": "Kimin tuttuğunu görmek için bırakın",
  "unlocker.pathLabel": "Yol",
  "unlocker.pathPlaceholder": "C:\\yol\\dosya.docx",
  "unlocker.addPath": "Ekle",
  "unlocker.pickError": "Dosya seçici açılamadı. Dosyaları buraya bırakın ya da yolu yazın.",
  "unlocker.picked": "Seçilen {count} yol kontrol ediliyor",
  "unlocker.clear": "Temizle",
  "unlocker.recheck": "Yeniden kontrol et",
  "unlocker.checking": "Bu dosyaları kimin tuttuğu Windows'a soruluyor…",
  "unlocker.queryError": "Windows bu soruyu yanıtlayamadı.",
  "unlocker.filesChecked": "{count} dosya kontrol edildi",
  "unlocker.capNote":
    "Bu klasörde WinKnife'ın tek seferde kontrol ettiğinden fazla dosya var: yalnızca ilk {cap} dosya kontrol edildi.",
  "unlocker.unreadable": "Klasördeki {count} öge okunamadı ve kontrol edilmedi.",
  "unlocker.skippedTitle": "Kontrol edilmedi",
  "unlocker.emptyTitle": "Bu dosyaları açık tutan bir şey yok.",
  "unlocker.emptyHint":
    "Dosya hâlâ değiştirilemiyorsa salt okunur olabilir ya da hesabınızın onu değiştirme izni olmayabilir.",
  "unlocker.pid": "PID {pid}",
  "unlocker.service": "hizmet {name}",
  "unlocker.kind.window_app": "Pencereli uygulama",
  "unlocker.kind.console": "Konsol",
  "unlocker.kind.service": "Hizmet",
  "unlocker.kind.explorer": "Gezgin",
  "unlocker.kind.critical": "Sistem için kritik",
  "unlocker.kind.unknown": "Bilinmiyor",
  "unlocker.holdsSome": "Bu dosyalardan {count} tanesini açık tutuyor.",
  "unlocker.holdsUnattributed": "Bu dosyalardan bir veya daha fazlasını tutuyor.",
  "unlocker.heldMore": "ve {count} tane daha",
  "unlocker.reason.window":
    "Pencereli uygulama. Oturumu kapatırken Windows'un yaptığı gibi kapanması istenebilir.",
  "unlocker.reason.console":
    "Konsol programı. Kapanması istenebilir; çalıştırdığı işi durdurabilir.",
  "unlocker.reason.service": "Windows hizmeti — eminseniz Hizmetler'den durdurun.",
  "unlocker.reason.explorer":
    "Gezgin — kapatmak yerine yeniden başlatmak daha güvenlidir; WinKnife bunu sizin yerinize yapmaz.",
  "unlocker.reason.critical":
    "Kritik sistem işlemi — kapatılması Windows'u çökertebilir ya da yeniden başlatabilir, bu yüzden WinKnife asla istemez.",
  "unlocker.reason.unknown":
    "Windows bunun ne tür bir program olduğunu söyleyemedi, bu yüzden WinKnife kapatmayı önermiyor.",
  "unlocker.reason.system":
    "Bunu Windows'un kendisi (System işlemi) tutuyor; genellikle dosya ağda paylaşıldığı ya da dizinlendiği için. Kapatılamaz.",
  "unlocker.reason.self": "Bu WinKnife'ın kendisi. Dosyayı kullanan ekranı kapatın.",
  "unlocker.askClose": "Kapanmasını iste",
  "unlocker.asking": "İsteniyor…",
  "unlocker.openLocation": "Dosya konumunu aç",
  "unlocker.confirmTitle": "{app} kapanması istensin mi?",
  "unlocker.confirmMessage":
    "{app} uygulamasından kapanması istenecek. Kaydedilmemiş işi varsa önce kaydetmenizi isteyebilir.",
  "unlocker.confirmNote": "WinKnife bir programı asla zorla kapatmaz ya da sonlandırmaz.",
  "unlocker.outcome.closed": "{app} kapandı.",
  "unlocker.outcome.still_running":
    "{app} hâlâ çalışıyor. Kaydetmenizi bekliyor olabilir ya da kapanmamayı seçti.",
  "unlocker.outcome.already_exited": "{app} zaten kapanmıştı.",
  "unlocker.outcome.refused": "{app} uygulamasından kapanması istenmedi: {reason}",
  "unlocker.closeError": "{app} uygulamasından kapanması istenemedi.",
};

/** Every key the dictionary knows. */
export type TranslationKey = keyof typeof en;

const dictionaries: Record<Locale, Partial<Record<string, string>>> = {
  en,
  tr,
};

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
