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
    "Every tool looks first and asks before it changes anything. Whatever you remove goes to quarantine, where you can get it back.",
  "dashboard.loading": "Loading your tools…",
  "dashboard.empty": "No tools are available yet.",
  "dashboard.error": "Could not read this PC's details right now.",
  "dashboard.retry": "Try again",
  "dashboard.reclaimable": "Space you could get back",
  "dashboard.reclaimableNote":
    "Based on the last time each tool ran. Nothing has been removed.",
  "dashboard.quarantineNote": "{count} items waiting in quarantine",
  "dashboard.timer": "Shutdown timer",
  "dashboard.timerActive": "Runs at {when}",
  "dashboard.free": "free",
  "dashboard.lastScan": "Last run {when} · {count} found",
  "dashboard.couldFree": "about {size} reclaimable",
  "dashboard.wasCancelled": "stopped early",
  "dashboard.neverRun": "Not run yet.",

  "module.open": "Open",
  "module.capability.scan": "Looks around",
  "module.capability.execute": "Can make changes",
  "module.capability.settings": "Has settings",
  "module.placeholder": "This tool is still being built.",

  "action.cancel": "Cancel",
  "action.close": "Close",

  "risk.safe": "Safe to remove",
  "risk.needsReview": "Needs your review",
  "risk.doNotDelete": "Left alone",

  "cleaner.intro":
    "The scan only looks. It checks temporary files, Explorer caches and the recycle bin, then shows you the list before anything moves.",
  "cleaner.scan": "Scan for junk",
  "cleaner.previousScan": "Last scan",
  "cleaner.historyNotice":
    "This is from an earlier scan. Scan again to see what is there now.",
  "cleaner.cameFrom":
    "You came here from {path} on the disk map. The cleaner checks known junk locations rather than a folder you choose. If that folder is one of them it will appear below; if not, nothing here touches it.",
  "cleaner.scanAgain": "Scan again",
  "cleaner.scanning": "Looking around…",
  "cleaner.itemsScanned": "{count} items looked at",
  "cleaner.analyzing": "Checking each item before suggesting anything…",
  "cleaner.analyzed": "{done} of {total} checked",
  "cleaner.stop": "Stop",
  "cleaner.stopping": "Stopping…",
  "cleaner.cancelled":
    "The scan was stopped early, so these lists are incomplete. Scan again for the full picture.",
  "cleaner.showMore": "Show {count} more ({hidden} not shown yet)",
  "cleaner.executing": "Moving files to quarantine…",
  "exec.progress": "{done} of {total}",
  "exec.stoppingNote":
    "Stopping after the current item. What has already moved stays in quarantine.",
  "exec.stopped":
    "Stopped early: {moved} moved to quarantine, {left} left where they were.",
  "cleaner.summary":
    "About {safe} can be reclaimed safely, and {review} needs your review.",
  "cleaner.skippedRules": "Some checks could not run on this PC.",
  "cleaner.collapseGroup": "Collapse {group}",
  "cleaner.expandGroup": "Expand {group}",
  "cleaner.groupCount": "{count} items",
  "cleaner.safeHint": "ticked for you",
  "cleaner.reviewHint": "choose one by one",
  "cleaner.blockedHint": "shown for information only",
  "cleaner.bucketEmpty": "Nothing in this group.",
  "cleaner.otherGroup": "Other",
  "cleaner.selectGroup": "Select everything in {group}",
  "cleaner.selection": "{count} items selected · {size}",
  "cleaner.clean": "Clean up",
  "cleaner.confirmTitle": "Move {count} items to quarantine?",
  "cleaner.confirmBody":
    "This frees about {size}. Nothing is deleted — the files move to quarantine first.",
  "cleaner.confirmNote":
    "Everything goes to quarantine first, not the bin. You can bring any of it back for the next {days} days; after that it is removed for good. Nothing is deleted right now.",
  "cleaner.confirmRecycleBin":
    "Your selection includes emptying the recycle bin. That part cannot be undone; everything else can.",
  "cleaner.doneTitle": "All done",
  "cleaner.doneBody":
    "Freed {size}. Everything that moved is in quarantine if you want it back.",
  "cleaner.openQuarantine": "Open quarantine",

  "nav.settings": "Settings",

  "duplicates.intro":
    "The scan reads your files and never changes them. Two files are only called duplicates when they match completely.",
  "duplicates.scan": "Find duplicates",
  "duplicates.pinnedRoot": "Only looking inside {path}, because you came here from the disk map.",
  "duplicates.clearPinnedRoot": "Use my saved folders instead",
  "duplicates.scanAgain": "Scan again",
  "duplicates.scanning": "Comparing files…",
  "duplicates.stop": "Stop",
  "duplicates.stopping": "Stopping…",
  "duplicates.cancelled":
    "Stopped before it finished, so nothing is listed. A partial list could hide copies that were not reached.",
  "duplicates.summary": "{size} taken up by {sets} sets of copies",
  "duplicates.empty": "No duplicates in the folders that were scanned.",
  "duplicates.hashed": "{count} files checked completely",
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
  "duplicates.unreadable":
    "Left out of the comparison because they could not be read: {count} files.",
  "duplicates.selection": "{count} copies selected · {size}",
  "duplicates.remove": "Move copies to quarantine",
  "duplicates.confirmTitle": "Move {count} copies to quarantine?",
  "duplicates.confirmBody":
    "This frees about {size}. The suggested keeper of every set stays where it is.",
  "duplicates.doneTitle": "Done",
  "duplicates.doneBody":
    "Freed {size}. Everything that moved is in quarantine if you want it back.",

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
  "treemap.staleness": "Mapped {when} · {size} in {files} files",
  "treemap.partial": "stopped early, so this is only part of the picture",
  "treemap.unreadable":
    "{count} folders could not be read, so some space is unaccounted for",
  "treemap.breadcrumb": "Folder path",
  "treemap.canvasLabel":
    "Disk map. Arrow keys move between folders, Enter opens one, Backspace goes up.",
  "treemap.empty":
    "This drive has not been mapped yet. Mapping only reads; nothing is changed.",
  "treemap.folderEmpty": "There is nothing in this folder.",
  "treemap.folderUnreadable":
    "This folder could not be opened, so what is inside is unknown.",
  "treemap.sizeUnknown": "size unknown",
  "treemap.sizeUnknownLong": "Size unknown: part of this folder could not be read.",
  "treemap.linkNote":
    "A shortcut folder. Its contents live elsewhere on this drive and are counted there.",
  "treemap.foldedNote": "{count} items grouped together",
  "treemap.compressedNote":
    "{count} files here are compressed, so they take up less room than shown.",
  "treemap.linksNote":
    "{count} shortcut folders are shown but not counted, so nothing is counted twice.",
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
    "Looking at this list changes nothing. Turning an entry off uses the same switch as Task Manager, so the two always agree. Nothing is deleted, and you can turn it back on here any time.",
  "startup.loading": "Reading what starts with Windows…",
  "startup.loadError": "Could not read what starts with Windows.",
  "startup.empty": "Nothing starts with Windows on this PC.",
  "startup.summary": "{total} entries · {on} start with Windows",
  "startup.refresh": "Refresh",
  "startup.unreadable":
    "Some places could not be read, so this list may be missing entries:",
  "startup.signed": "signed",
  "startup.unsigned": "unsigned",
  "startup.impact": "{tier} impact on startup (estimated)",
  "startup.impactLow": "Low",
  "startup.impactMedium": "Medium",
  "startup.impactHigh": "High",
  "startup.verdictKeep": "Best left on",
  "startup.verdictUserChoice": "Your call",
  "startup.verdictSafeToDisable": "Safe to turn off",
  "startup.verdictUnknown": "Not recognised",
  "startup.ratingSafe": "Safe to turn off",
  "startup.ratingReview": "Review before turning off",
  "startup.ratingWhy": "Why this rating?",
  "startup.on": "On",
  "startup.off": "Off",
  "startup.offLeftover": "Off — but its shortcut is still in the Startup folder",
  "startup.leftoverReason":
    "WinKnife has this turned off, but a shortcut with the same name is back at {path}, so it still runs. Turning it on keeps that shortcut; if WinKnife's copy is identical it goes to quarantine, otherwise nothing changes and you choose.",
  "startup.working": "Working…",
  "startup.toggleLabel": "Start {name} with Windows",
  "startup.confirmTitle": "Stop {name} starting with Windows?",
  "startup.confirmBody":
    "It will not start next time you log in. You can still open it yourself whenever you like.",
  "startup.confirmNote":
    "Nothing is deleted. Windows is told to skip this entry, the same way Task Manager does it, and a shortcut is kept in WinKnife's own folder so it can be put back unchanged.",
  "startup.confirmAction": "Turn it off",

  "settings.title": "Settings",
  "settings.subtitle":
    "Everything here stays on this PC. If a value is outside the safe range it is adjusted when you save, and the page shows what was kept.",
  "settings.loading": "Loading settings…",
  "settings.error": "Could not load your settings.",
  "settings.save": "Save settings",
  "settings.saving": "Saving…",
  "settings.saved": "Saved.",
  "settings.adjusted":
    "Some values were outside the allowed range and have been adjusted.",
  "settings.packsTitle": "What the cleaner checks",
  "settings.packsSubtitle": "A check that is off is skipped completely.",
  "settings.packDefaultOn": "on by default",
  "settings.packDefaultOff": "off by default",
  "settings.packRules": "{count} checks",
  "settings.duplicatesTitle": "Duplicate finder",
  "settings.minSize": "Ignore files smaller than",
  "settings.minSizeHint":
    "Below {size} the scan mostly finds noise, so that is the smallest allowed.",
  "settings.quarantineTitle": "Quarantine",
  "settings.retentionDays": "Keep removed files for",
  "settings.retentionHint":
    "Between {min} and {max} days. Zero is not allowed, because everything would be removed for good at the next start.",
  "settings.maxBytes": "Keep quarantine under",
  "settings.maxBytesHint":
    "Encrypted files that could not be restored do not count towards this limit; only you can remove them.",
  "settings.days": "days",

  "quarantine.title": "Quarantine",
  "quarantine.subtitle":
    "Anything a tool removed waits here. Restore it any time until the keeping period runs out.",
  "quarantine.open": "Quarantine",
  "quarantine.held": "{count} items · {size} held",
  "quarantine.loading": "Loading…",
  "quarantine.empty": "Nothing is waiting in quarantine.",
  "quarantine.batchLabel": "{count} items",
  "quarantine.keptFor": "Kept for another {time}",
  "quarantine.restore": "Restore",
  "quarantine.showItems": "Show items",
  "quarantine.hideItems": "Hide items",
  "quarantine.purgeNow": "Remove for good now",
  "quarantine.unreadableRows":
    "{count} entries in the quarantine record can no longer be read, so they are missing from the totals. Their files are still in the quarantine folder and have not been touched.",
  "quarantine.purged": "Removed {count} items for good and freed {size}.",
  "quarantine.affectedBatches":
    "This came from {count} groups. The size limit removes the oldest items first, so a group can shrink and still be listed.",
  "quarantine.purgeFailed":
    "{count} items could not be removed. They will be tried again next time.",
  "quarantine.restored": "Restored {count} items.",
  "quarantine.restoring": "Restoring…",
  "quarantine.restoreStopped":
    "Stopped: {count} restored, the rest is still in quarantine.",
  "quarantine.restoreRefused":
    "Could not restore: the file is encrypted. It stays here until you remove it.",
  "quarantine.removeItem": "Remove",
  "quarantine.removeTitle": "Remove this item for good?",
  "quarantine.removeBody":
    "{path} could not be restored because it is encrypted, so it is kept here until you remove it.",
  "quarantine.removeWarning": "Removing it deletes it permanently. This cannot be undone.",
  "quarantine.removeNote":
    "To keep it instead, decrypt it where it is held (the path below: Properties, Advanced, clear “Encrypt contents”) and restore it again.",
  "quarantine.heldAt": "Held at",
  "quarantine.removeFailed":
    "{path} could not be removed: its file could not be deleted. It is still kept, and is never removed automatically; try again, or decrypt it there and restore it.",
  "quarantine.removeRefused":
    "{path} was not removed: it is no longer held here. Refresh the list to see where it stands.",
  "quarantine.removeNotMarked":
    "{path} was not removed: its restore was never refused, so it is not an item you remove by hand. It stays until its keeping period ends, and you can still restore it.",
  "quarantine.removeError": "{path} could not be removed: {error}.",
  "quarantine.refusedHeld":
    "Encrypted items kept after a refused restore: {count}, using {used}. They do not count towards the {cap} size limit, and only you can remove them: remove them here, or decrypt them and restore them.",
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
    "WinKnife keeps the countdown itself, so it needs to stay open. The icon in the tray shows the time left and can cancel the timer.",
  "shutdown.dryRunBanner":
    "This copy was started in dry-run mode. The timer counts down and warns you, but nothing is actually shut down.",

  "restorePoint.statusLabel": "System Protection",
  "restorePoint.statusLoading": "Checking System Protection…",
  "restorePoint.protection.on": "Looks on",
  "restorePoint.protection.off": "Looks off",
  "restorePoint.protection.unknown": "Could not tell",
  "restorePoint.descriptionLabel": "Description (optional)",
  "restorePoint.descriptionHint":
    "Up to {max} characters. Leave it empty to use the default name.",
  "restorePoint.uacNotice":
    "Windows will ask for permission (the UAC prompt) because creating a restore point needs administrator rights. WinKnife does not change your System Protection settings.",
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
    "System Protection is turned on in Windows' own settings. WinKnife only opens that window for you.",
  "restorePoint.openSettings": "Open System Protection settings",
  "startup.restorePointLink": "Create a restore point first",
  // Same text as the Rust manifest; listed so Turkish can override it.
  "module.restore-point.name": "Restore Point",
  "module.restore-point.description":
    "Create a Windows restore point before you change system settings.",
  "legacyPanels.loading": "Finding the panels on this PC…",
  "legacyPanels.group.system": "System",
  "legacyPanels.group.hardware": "Hardware & devices",
  "legacyPanels.group.network": "Network",
  "legacyPanels.group.apps": "Apps & accounts",
  "legacyPanels.group.diagnostics": "Diagnostics",
  "legacyPanels.open": "Open",
  "legacyPanels.unavailable": "Not available on this edition of Windows.",
  "legacyPanels.notice":
    "These are Windows' own windows; WinKnife only opens them. Some will ask for administrator permission.",
  "module.legacy-panels.name": "Legacy Panels",
  "module.legacy-panels.description":
    "Open the classic Windows windows that Settings hides: Device Manager, Network Connections, Services and more.",
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
  "keepAwake.idle": "The PC can sleep as usual.",
  "keepAwake.activeFor": "Staying awake for",
  "keepAwake.until": "Until {time}",
  "keepAwake.activeDisplay": "The screen stays on as well.",
  "keepAwake.noPersist":
    "This is not saved. Closing WinKnife or restarting the PC ends it. The longest you can set is 24 hours.",
  "keepAwake.shutdownArmed":
    "A shutdown timer is set. Keeping the PC awake does not cancel it: it will still shut down, restart or sleep when the timer ends.",
  "keepAwake.invalidMinutes": "Enter a whole number between 1 and 1440.",

  "emptyFolders.intro":
    "Finds folders with nothing in them, however deep. Nothing starts ticked, and anything you remove goes to quarantine first.",
  "emptyFolders.rootLabel": "Folder to scan",
  "emptyFolders.rootHint": "Leave this empty to check your user folder.",
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
    "See which program has a file open. Looking is read-only, and a program is only ever asked to close, never forced.",
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
    "This folder has a lot of files, so only the first {cap} were checked.",
  "unlocker.unreadable": "{count} entries inside the folder could not be read and were not checked.",
  "unlocker.skippedTitle": "Not checked",
  "unlocker.emptyTitle": "No program is holding these files open.",
  "unlocker.emptyHint":
    "If the file still cannot be changed, it may be read-only, or your account may not have permission to change it.",
  "unlocker.pid": "Process {pid}",
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

  "status.version": "Version {version}",

  // --- appearance (WP-B) ---
  "settings.appearanceTitle": "Appearance",
  "settings.appearanceSubtitle":
    "Pick the look you find easiest on the eyes. It changes right away and is remembered on this PC.",
  "settings.themeLabel": "Theme",
  "settings.theme.graphite": "Graphite",
  "settings.themeHint.graphite": "Dark, neutral grey",
  "settings.theme.forest": "Forest",
  "settings.themeHint.forest": "Dark, deep green",
  "settings.theme.mist": "Mist",
  "settings.themeHint.mist": "Light, soft grey",
  "settings.themeNote":
    "The theme is saved on its own. The button below is for the other settings.",
  "settings.animationsLabel": "Animations",
  "settings.animationsHint": "Moving progress bars and other motion. Saved on its own.",
  "settings.animationsReduced":
    "Off while Windows asks apps to reduce motion (Settings › Accessibility › Visual effects).",
  "settings.animationsOn": "On",
  "settings.animationsOff": "Off",
  "settings.glowLabel": "Background light",
  "settings.glowHint": "Moves slowly and leans toward the mouse. Still when animations are off.",
  "settings.glow.aurora": "Aurora",
  "settings.glow.waves": "Waves",
  "settings.glow.spotlight": "Spotlight",
  "nav.allToolsHidden": "Every tool is hidden. Show them again in Settings.",
  "settings.sidebarTitle": "Tools in the sidebar",
  "settings.sidebarSubtitle":
    "Untick a tool you do not use to take it out of the sidebar. It stays on the dashboard and still opens from there. Saved on this machine.",
  // --- copy audit (WP-C) ---
  "dashboard.status.default": "Ready when you are.",
  "dashboard.status.file-unlocker": "Drop a file here to see what is holding it.",
  "dashboard.status.keep-awake": "The PC sleeps as usual right now.",
  "dashboard.status.legacy-panels":
    "Device Manager, Services and other classic windows.",
  "dashboard.status.restore-point": "A safety net before you change Windows settings.",
  "dashboard.status.shutdown-timer": "No timer is set.",
  "dashboard.status.startup": "See what starts with Windows.",
  "dashboard.action.cleaner": "Scan for junk",
  "dashboard.action.treemap": "Map a drive",
  "dashboard.action.duplicates": "Find duplicates",
  "dashboard.action.empty-folders": "Find empty folders",
  "dashboard.action.startup": "Review startup",
  "dashboard.action.file-unlocker": "Check a file",
  "dashboard.action.keep-awake": "Keep the PC awake",
  "dashboard.action.legacy-panels": "Open a panel",
  "dashboard.action.restore-point": "Create a restore point",
  "dashboard.action.shutdown-timer": "Set a timer",
  "module.duplicates.description":
    "Find files that are exact copies of each other, pick which one to keep, and tidy the rest away.",
  "module.startup.description":
    "See what starts with Windows and switch things off the same way Task Manager does.",
  "duplicates.minSize.any": "Any size",
  "duplicates.noDate": "No date",
  "duplicates.how": "How it checks",
  "duplicates.howBody":
    "Sizes are compared first, then the start and end of each file, then the whole file.",
  "cleaner.skippedRulesDetails": "Which checks",
  "cleaner.cameFromSummary": "Why is my folder not listed?",
  "startup.leftoverSummary": "Why is this listed twice?",
  "restorePoint.uacSummary": "Why does Windows ask for permission?",
  "quarantine.detailsSummary": "Details",
  "about.title": "About WinKnife",
  "about.official": "The only official source is github.com/armadavalor/WinKnife. Releases there list a SHA-256 for every file; compare it with the one you computed.",
  "about.neverTitle": "What WinKnife never does",
  "about.never.network": "Makes no network connection unless you turn on an optional feature. Nothing is sent anywhere, and this page fetches nothing.",
  "about.never.update": "Does not update itself. You install new versions yourself.",
  "about.never.service": "Runs no background service. When the window closes, WinKnife stops.",
  "about.never.delete": "Never deletes your files outright. Cleanup moves them to quarantine, where you can restore them; they are purged only when the retention age or size limit you set is reached. The one exception is emptying the Windows recycle bin: that is permanent, needs its own confirmation, and starts unticked.",
  "about.verify": "How to verify a download: see \"Verify this download\" in the README on GitHub.",
} as const;

/** Turkish overrides. Missing keys fall back to English. */
const tr: Partial<Record<TranslationKey, string>> = {
  "quarantine.removeNotMarked":
    "{path} kaldırılmadı: geri yüklemesi hiç reddedilmedi, bu yüzden elle kaldırılan bir öğe değil. Saklama süresi bitene kadar kalır ve hâlâ geri yükleyebilirsiniz.",
  "status.version": "Sürüm {version}",
  "restorePoint.statusLabel": "Sistem Koruması",
  "restorePoint.statusLoading": "Sistem Koruması kontrol ediliyor…",
  "restorePoint.protection.on": "Açık görünüyor",
  "restorePoint.protection.off": "Kapalı görünüyor",
  "restorePoint.protection.unknown": "Anlaşılamadı",
  "restorePoint.descriptionLabel": "Açıklama (isteğe bağlı)",
  "restorePoint.descriptionHint":
    "En fazla {max} karakter. Boş bırakırsanız varsayılan ad kullanılır.",
  "restorePoint.uacNotice":
    "Windows izin isteyecek (UAC penceresi), çünkü geri yükleme noktası oluşturmak yönetici hakları gerektirir. WinKnife Sistem Koruması ayarlarınızı değiştirmez.",
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
    "Sistem Koruması, Windows'un kendi ayarlarından açılır. WinKnife yalnızca o pencereyi sizin için açar.",
  "restorePoint.openSettings": "Sistem Koruması ayarlarını aç",
  "startup.restorePointLink": "Önce bir geri yükleme noktası oluşturun",
  "module.restore-point.name": "Geri Yükleme Noktası",
  "module.restore-point.description":
    "Sistem ayarlarını değiştirmeden önce bir Windows geri yükleme noktası oluşturun.",
  "startup.offLeftover": "Kapalı — ama kısayolu hâlâ Başlangıç klasöründe",
  "startup.leftoverReason":
    "WinKnife bunu kapalı tutuyor, ancak aynı adlı bir kısayol {path} konumuna geri gelmiş; bu yüzden hâlâ çalışıyor. Açarsanız o kısayol kalır; WinKnife'ın kopyası aynıysa karantinaya gider, değilse hiçbir şey değişmez ve siz seçersiniz.",
  "module.empty-folders.name": "Boş klasörler",
  "module.empty-folders.description":
    "Hiçbir derinlikte dosya içermeyen klasörleri bulur ve karantinaya taşır.",
  "emptyFolders.intro":
    "İçinde hiçbir şey olmayan klasörleri, ne kadar derinde olursa olsun bulur. Hiçbiri baştan işaretli gelmez; kaldırdığınız her şey önce karantinaya gider.",
  "emptyFolders.rootLabel": "Taranacak klasör",
  "emptyFolders.rootHint": "Kullanıcı klasörünüzü kontrol etmek için boş bırakın.",
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
    "Bu kaydedilmez. WinKnife'ı kapatmak ya da bilgisayarı yeniden başlatmak bunu sonlandırır. En fazla 24 saat ayarlanabilir.",
  "keepAwake.shutdownArmed":
    "Bir kapatma zamanlayıcısı kurulu. Uyanık tutmak onu iptal etmez: zamanlayıcı bitince bilgisayar yine kapanır, yeniden başlar ya da uykuya geçer.",
  "keepAwake.invalidMinutes": "1 ile 1440 arasında bir tam sayı girin.",
  "unlocker.title": "Dosya Kilidi Açıcı",
  "unlocker.subtitle":
    "Bir dosyayı hangi programın açık tuttuğunu görün. Bakmak salt okunurdur; bir programdan yalnızca kapanması istenir, asla zorlanmaz.",
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
    "Bu klasörde çok sayıda dosya var, bu yüzden yalnızca ilk {cap} dosya kontrol edildi.",
  "unlocker.unreadable": "Klasördeki {count} öge okunamadı ve kontrol edilmedi.",
  "unlocker.skippedTitle": "Kontrol edilmedi",
  "unlocker.emptyTitle": "Bu dosyaları açık tutan bir program yok.",
  "unlocker.emptyHint":
    "Dosya hâlâ değiştirilemiyorsa salt okunur olabilir ya da hesabınızın onu değiştirme izni olmayabilir.",
  "unlocker.pid": "İşlem {pid}",
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

  // --- appearance (WP-B) ---
  "settings.appearanceTitle": "Görünüm",
  "settings.appearanceSubtitle":
    "Gözünüze en rahat gelen görünümü seçin. Hemen uygulanır ve bu bilgisayarda hatırlanır.",
  "settings.themeLabel": "Tema",
  "settings.theme.graphite": "Grafit",
  "settings.themeHint.graphite": "Koyu, nötr gri",
  "settings.theme.forest": "Orman",
  "settings.themeHint.forest": "Koyu, derin yeşil",
  "settings.theme.mist": "Sis",
  "settings.themeHint.mist": "Açık, yumuşak gri",
  "settings.themeNote":
    "Tema kendi başına kaydedilir. Aşağıdaki düğme diğer ayarlar içindir.",
  "settings.animationsLabel": "Animasyonlar",
  "settings.animationsHint": "Hareketli ilerleme çubukları ve diğer hareketler. Kendi başına kaydedilir.",
  "settings.animationsReduced":
    "Windows uygulamalardan hareketi azaltmalarını istediği sürece kapalı (Ayarlar › Erişilebilirlik › Görsel efektler).",
  "settings.animationsOn": "Açık",
  "settings.animationsOff": "Kapalı",
  "settings.glowLabel": "Arka plan ışığı",
  "settings.glowHint": "Yavaşça hareket eder ve fareye doğru kayar. Animasyonlar kapalıyken durur.",
  "settings.glow.aurora": "Kutup ışığı",
  "settings.glow.waves": "Dalgalar",
  "settings.glow.spotlight": "Spot ışığı",
  "nav.allToolsHidden": "Tüm araçlar gizli. Ayarlar'dan yeniden gösterebilirsin.",
  "settings.sidebarTitle": "Kenar çubuğundaki araçlar",
  "settings.sidebarSubtitle":
    "Kullanmadığın bir aracın işaretini kaldırırsan kenar çubuğundan çıkar. Panoda kalır ve oradan yine açılır. Bu makineye kaydedilir.",
  // --- copy audit (WP-C) ---
  "dashboard.status.default": "Hazır olduğunuzda başlayabilirsiniz.",
  "dashboard.action.cleaner": "Gereksizleri tara",
  "dashboard.action.treemap": "Bir sürücüyü haritala",
  "dashboard.action.duplicates": "Kopyaları bul",
  "dashboard.action.empty-folders": "Boş klasörleri bul",
  "dashboard.action.startup": "Başlangıcı incele",
  "dashboard.action.file-unlocker": "Bir dosyayı kontrol et",
  "dashboard.action.keep-awake": "Bilgisayarı uyanık tut",
  "dashboard.action.legacy-panels": "Bir panel aç",
  "dashboard.action.restore-point": "Geri yükleme noktası oluştur",
  "dashboard.action.shutdown-timer": "Zamanlayıcı kur",
  "duplicates.minSize.any": "Herhangi bir boyut",
  "duplicates.noDate": "Tarih yok",
  "duplicates.how": "Nasıl kontrol eder",
  "duplicates.howBody":
    "Önce boyutlar, sonra her dosyanın başı ve sonu, en son da dosyanın tamamı karşılaştırılır.",
  "cleaner.skippedRulesDetails": "Hangi kontroller",
  "cleaner.analyzing": "Bir şey önermeden önce her öğe kontrol ediliyor…",
  "cleaner.analyzed": "{total} öğenin {done} tanesi kontrol edildi",
  "cleaner.stop": "Durdur",
  "cleaner.stopping": "Durduruluyor…",
  "cleaner.executing": "Dosyalar karantinaya taşınıyor…",
  "exec.progress": "{done} / {total}",
  "exec.stoppingNote":
    "Geçerli öğeden sonra duracak. Taşınmış olanlar karantinada kalır.",
  "exec.stopped":
    "Erken durduruldu: {moved} öğe karantinaya taşındı, {left} öğe yerinde bırakıldı.",
  "quarantine.restoring": "Geri yükleniyor…",
  "quarantine.restoreStopped":
    "Durduruldu: {count} öğe geri yüklendi, kalanı hâlâ karantinada.",
  "cleaner.cancelled":
    "Tarama erken durduruldu, bu yüzden listeler eksik. Tam sonuç için yeniden tarayın.",
  "cleaner.showMore": "{count} tane daha göster ({hidden} henüz gösterilmiyor)",
  "cleaner.cameFromSummary": "Klasörüm neden listelenmiyor?",
  "startup.leftoverSummary": "Bu neden iki kez listeleniyor?",
  "startup.ratingSafe": "Kapatmak güvenli",
  "startup.ratingReview": "Kapatmadan önce gözden geçirin",
  "startup.ratingWhy": "Bu değerlendirme neden?",
  "restorePoint.uacSummary": "Windows neden izin istiyor?",
  "quarantine.detailsSummary": "Ayrıntılar",
  "about.title": "WinKnife hakkında",
  "about.official": "Tek resmî kaynak github.com/armadavalor/WinKnife. Oradaki sürümler her dosya için SHA-256 listeler; hesapladığınla karşılaştır.",
  "about.neverTitle": "WinKnife'ın asla yapmadıkları",
  "about.never.network": "İsteğe bağlı bir özelliği açmadıkça ağ bağlantısı kurmaz. Hiçbir yere bir şey göndermez, bu sayfa da hiçbir şey indirmez.",
  "about.never.update": "Kendini güncellemez. Yeni sürümleri sen kurarsın.",
  "about.never.service": "Arka planda hizmet çalıştırmaz. Pencere kapanınca WinKnife durur.",
  "about.never.delete": "Dosyalarını doğrudan silmez. Temizlik onları geri yükleyebileceğin karantinaya taşır; yalnız belirlediğin saklama süresi veya boyut sınırı dolunca temizlenir. Tek istisna Windows geri dönüşüm kutusunu boşaltmak: kalıcıdır, ayrı onay ister ve tiksiz başlar.",
  "about.verify": "İndirmeyi doğrulamak için GitHub'daki README'de \"Verify this download\" bölümüne bak.",
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
