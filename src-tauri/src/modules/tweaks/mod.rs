//! Reversible per-user settings (Phase 6, K-018).
//!
//! A small, fixed catalogue of `HKCU` values. Before anything is written the
//! prior value — or the fact that there was none — goes into `tweak_undo`, the
//! write is read back, and undo puts back exactly those bytes. A tweak whose
//! undo cannot be shown to work is not shipped; the fence below refuses it in
//! code, not just in review.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::core::error::{CoreError, Result};
use crate::core::executor::TweakControl;
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::platform::{Hive, RegValue, Registry};
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket, MAX_CONFIDENCE,
};

pub mod profile;
pub mod update_block;

/// What has to happen before a changed setting shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Restart {
    /// It shows straight away.
    None,
    /// Open Explorer windows pick it up after a refresh or a restart of
    /// Explorer.
    Explorer,
    /// It needs a sign-out or a reboot.
    Reboot,
}

/// How far an applied tweak can be taken back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Undoability {
    /// Undo restores the value and it shows at once.
    Reversible,
    /// Undo restores the value; it shows after the same restart the apply
    /// needed.
    ReversibleAfterRestart,
    /// There is no proven undo. The fence refuses these; the variant exists so
    /// the refusal is something a test can pin.
    NotUndoable,
}

/// Where a tweak sits on the Tweaks page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TweakCategory {
    Explorer,
    TaskbarStart,
    Privacy,
    Input,
    Appearance,
}

/// One catalogue entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tweak {
    /// Stable id, used by the UI, the executor and the undo table.
    pub id: &'static str,
    /// Plain-language name.
    pub name: &'static str,
    /// Why someone would want it, shown as the recommendation's reason.
    pub reason: &'static str,
    /// Page section.
    pub category: TweakCategory,
    /// Which Windows versions the setting does anything on.
    pub applies_to: &'static str,
    /// Always [`Hive::CurrentUser`]; anything else is fenced off.
    pub hive: Hive,
    /// Key under the hive.
    pub subkey: &'static str,
    /// Value name under the key.
    pub value_name: &'static str,
    /// The `REG_DWORD` the tweak writes.
    pub target: u32,
    /// Where the setting is documented.
    pub source_url: &'static str,
    /// What has to happen before it shows.
    pub restart: Restart,
    /// How far it can be taken back.
    pub undo: Undoability,
}

const EXPLORER_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer";
const EXPLORER_ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
const CONTENT_DELIVERY: &str = r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager";
const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const PRIVACY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Privacy";
const ADVERTISING_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo";
const LANGUAGE_KEY: &str = r"Control Panel\International\User Profile";

const FILE_EXPLORER_DOCS: &str = "https://support.microsoft.com/en-us/windows/file-explorer-in-windows-ef370130-1cca-9dc5-e0df-2f7416fe1cb1";
const TASKBAR_DOCS: &str = "https://support.microsoft.com/en-us/windows/customize-the-taskbar-in-windows-0657a50f-0cc7-dbfd-ae6b-05020b195b07";
const PRIVACY_DOCS: &str = "https://learn.microsoft.com/en-us/windows/privacy/manage-connections-from-windows-operating-system-components-to-microsoft-services";
const SNAP_DOCS: &str = "https://support.microsoft.com/en-us/windows/snap-your-windows-885a9b1e-a983-a3b1-16cd-c531795e6241";
const PERSONALIZE_DOCS: &str = "https://support.microsoft.com/en-us/windows/change-desktop-background-and-colors-in-windows-3d5aa4d4-4a87-4fe4-a66f-5b8a1dd8b6bb";

const BOTH: &str = "Windows 10 and 11";
const WIN11: &str = "Windows 11 only";

use Restart::{Explorer as RestartExplorer, None as NoRestart};
use TweakCategory::{Appearance, Explorer, Input, Privacy, TaskbarStart};

/// A `REG_DWORD` tweak under `HKCU`; the undo class follows from the restart.
#[allow(clippy::too_many_arguments)]
const fn dword(
    id: &'static str,
    name: &'static str,
    category: TweakCategory,
    applies_to: &'static str,
    subkey: &'static str,
    value_name: &'static str,
    target: u32,
    restart: Restart,
    source_url: &'static str,
    reason: &'static str,
) -> Tweak {
    Tweak {
        id,
        name,
        reason,
        category,
        applies_to,
        hive: Hive::CurrentUser,
        subkey,
        value_name,
        target,
        source_url,
        restart,
        undo: match restart {
            Restart::None => Undoability::Reversible,
            _ => Undoability::ReversibleAfterRestart,
        },
    }
}

/// The shipped catalogue: per-user `REG_DWORD` values that Windows' own
/// Settings or Folder Options flip, so each can also be put back by hand. The
/// first three keep their place (tests index them); the page groups by
/// category.
pub const CATALOG: &[Tweak] = &[
    dword("show-file-extensions", "Show file name extensions", Explorer, BOTH,
        EXPLORER_ADVANCED, "HideFileExt", 0, RestartExplorer,
        "https://support.microsoft.com/en-us/windows/common-file-name-extensions-in-windows-da4a4430-8e76-89c5-59f7-1cdbbc75cb01",
        "Shows the .exe, .pdf or .docx at the end of file names, so a file called invoice.pdf.exe cannot pass for a document."),
    dword("show-hidden-files", "Show hidden files and folders", Explorer, BOTH,
        EXPLORER_ADVANCED, "Hidden", 1, RestartExplorer, FILE_EXPLORER_DOCS,
        "Lists files and folders marked hidden, such as AppData, so you can see what is taking up space. Windows system files stay hidden."),
    dword("open-explorer-to-this-pc", "Open File Explorer to This PC", Explorer, BOTH,
        EXPLORER_ADVANCED, "LaunchTo", 1, RestartExplorer, FILE_EXPLORER_DOCS,
        "New File Explorer windows start on your drives instead of Home or Quick access."),
    dword("explorer-item-checkboxes", "Tick boxes to select items", Explorer, BOTH,
        EXPLORER_ADVANCED, "AutoCheckSelect", 1, RestartExplorer, FILE_EXPLORER_DOCS,
        "Adds a checkbox to each file so you can select several without holding Ctrl; handy on touch screens and for people who mis-click."),
    dword("explorer-hide-recent-files", "Hide recent files in Quick access", Explorer, BOTH,
        EXPLORER_KEY, "ShowRecent", 0, RestartExplorer, FILE_EXPLORER_DOCS,
        "Stops File Explorer listing the files you opened last, which others can read over your shoulder. Existing history is not erased."),
    dword("explorer-hide-frequent-folders", "Hide frequent folders in Quick access", Explorer, BOTH,
        EXPLORER_KEY, "ShowFrequent", 0, RestartExplorer, FILE_EXPLORER_DOCS,
        "Stops File Explorer pinning the folders you visit most to Quick access. Existing history is not erased."),
    dword("explorer-no-sync-notices", "Hide OneDrive and Microsoft 365 notices in Explorer", Explorer, BOTH,
        EXPLORER_ADVANCED, "ShowSyncProviderNotifications", 0, RestartExplorer, FILE_EXPLORER_DOCS,
        "Turns off the banners in File Explorer that suggest OneDrive or Microsoft 365. It is the Folder Options box \"Show sync provider notifications\"."),
    dword("taskbar-hide-task-view", "Hide the Task View button", TaskbarStart, BOTH,
        EXPLORER_ADVANCED, "ShowTaskViewButton", 0, RestartExplorer, TASKBAR_DOCS,
        "Removes a button that Win+Tab already opens, giving the taskbar room."),
    dword("taskbar-hide-widgets", "Hide the Widgets button", TaskbarStart, WIN11,
        EXPLORER_ADVANCED, "TaskbarDa", 0, RestartExplorer, TASKBAR_DOCS,
        "Removes the weather and news button from the taskbar. Some recent builds protect this value; if Windows refuses it, WinKnife leaves the old value in place."),
    dword("taskbar-hide-copilot", "Hide the Copilot button", TaskbarStart, WIN11,
        EXPLORER_ADVANCED, "ShowCopilotButton", 0, RestartExplorer, TASKBAR_DOCS,
        "Removes the Copilot button from the taskbar (Windows 11 23H2 and later)."),
    dword("taskbar-left-align", "Align taskbar icons to the left", TaskbarStart, WIN11,
        EXPLORER_ADVANCED, "TaskbarAl", 0, RestartExplorer, TASKBAR_DOCS,
        "Moves the Start button and icons to the left edge, where they were in Windows 10, so the Start button stays put as apps open."),
    dword("taskbar-clock-seconds", "Show seconds in the taskbar clock", TaskbarStart, BOTH,
        EXPLORER_ADVANCED, "ShowSecondsInSystemClock", 1, RestartExplorer, TASKBAR_DOCS,
        "Adds seconds to the clock. It costs a little more battery and CPU than a clock that ticks once a minute."),
    dword("start-suggestions", "Hide suggested apps in Start", TaskbarStart, BOTH,
        CONTENT_DELIVERY, "SystemPaneSuggestionsEnabled", 0, NoRestart, PRIVACY_DOCS,
        "Stops Start from showing app suggestions and promotions. It is Settings > Personalization > Start > Show suggestions."),
    dword("start-silent-app-installs", "Stop promoted apps installing themselves", TaskbarStart, BOTH,
        CONTENT_DELIVERY, "SilentInstalledAppsEnabled", 0, NoRestart, PRIVACY_DOCS,
        "Stops Windows quietly adding promoted apps to Start. Apps already added stay until you remove them."),
    dword("lockscreen-fun-facts", "Hide lock screen tips and fun facts", Privacy, BOTH,
        CONTENT_DELIVERY, "RotatingLockScreenOverlayEnabled", 0, NoRestart, PRIVACY_DOCS,
        "Removes the \"Get fun facts, tips and more\" text over the Spotlight lock screen picture."),
    dword("tips-notifications", "Turn off \"Get tips\" notifications", Privacy, BOTH,
        CONTENT_DELIVERY, "SubscribedContent-338389Enabled", 0, NoRestart, PRIVACY_DOCS,
        "Stops pop-ups with tips and suggestions about Windows. It is Settings > System > Notifications > Get tips and suggestions."),
    dword("advertising-id", "Turn off the advertising ID", Privacy, BOTH,
        ADVERTISING_KEY, "Enabled", 0, NoRestart, PRIVACY_DOCS,
        "Stops apps using one ID to tailor ads to you across apps. You still see ads, just less targeted ones."),
    dword("tailored-experiences", "Turn off tailored experiences", Privacy, BOTH,
        PRIVACY_KEY, "TailoredExperiencesWithDiagnosticDataEnabled", 0, NoRestart, PRIVACY_DOCS,
        "Stops Microsoft using your diagnostic data to pick tips and ads for you. It is Settings > Privacy > Diagnostics & feedback > Tailored experiences."),
    dword("websites-language-list", "Hide your language list from websites", Privacy, BOTH,
        LANGUAGE_KEY, "HttpAcceptLanguageOptOut", 1, NoRestart, PRIVACY_DOCS,
        "Stops websites reading the list of languages you have installed, a detail that helps fingerprint your PC."),
    dword("snap-assist", "Turn off Snap Assist suggestions", Input, BOTH,
        EXPLORER_ADVANCED, "SnapAssist", 0, NoRestart, SNAP_DOCS,
        "After you snap a window to one side, Windows stops filling the other half with thumbnails of your open windows."),
    dword("snap-layout-flyout", "Turn off the snap layouts pop-up", Input, WIN11,
        EXPLORER_ADVANCED, "EnableSnapAssistFlyout", 0, NoRestart, SNAP_DOCS,
        "Stops the layout grid appearing when the pointer rests on a window's maximize button. Win+Z still opens it."),
    dword("window-shake", "Turn off shake to minimize", Input, BOTH,
        EXPLORER_ADVANCED, "DisallowShaking", 1, NoRestart, SNAP_DOCS,
        "Dragging a window back and forth no longer minimizes all the others, which is easy to trigger by accident."),
    dword("alt-tab-windows-only", "Alt+Tab shows windows only, not Edge tabs", Input, WIN11,
        EXPLORER_ADVANCED, "MultiTaskingAltTabFilter", 3, NoRestart, SNAP_DOCS,
        "Alt+Tab lists open windows only, instead of mixing in every Microsoft Edge tab."),
    dword("dark-mode-apps", "Dark mode for apps", Appearance, BOTH,
        PERSONALIZE, "AppsUseLightTheme", 0, NoRestart, PERSONALIZE_DOCS,
        "Apps that follow the system setting switch to their dark look, easier on the eyes at night."),
    dword("dark-mode-system", "Dark mode for Windows", Appearance, BOTH,
        PERSONALIZE, "SystemUsesLightTheme", 0, NoRestart, PERSONALIZE_DOCS,
        "The taskbar, Start and system dialogs switch to their dark look."),
    dword("disable-transparency", "Turn off transparency effects", Appearance, BOTH,
        PERSONALIZE, "EnableTransparency", 0, NoRestart, PERSONALIZE_DOCS,
        "Makes the taskbar and Start solid. A little easier to read, and slightly lighter on weak graphics hardware."),
];

/// Path fragments a tweak may never touch (K-018): Windows Update, Defender,
/// search, the Start menu, and policy keys that belong to an administrator.
/// Matched against the lower-cased `subkey\value`.
const FENCED_FRAGMENTS: &[&str] = &[
    "windowsupdate",
    "defender",
    "windows search",
    r"\search",
    "start_",
    "startpage",
    "startmenu",
    r"\policies\",
];

/// Why `tweak` may never be applied, or `None` when it is inside the fence.
pub fn fence_refusal(tweak: &Tweak) -> Option<String> {
    if tweak.hive != Hive::CurrentUser {
        return Some(format!("`{}` writes outside HKCU", tweak.id));
    }
    if tweak.undo == Undoability::NotUndoable {
        return Some(format!("`{}` has no proven undo", tweak.id));
    }
    let path = format!(r"{}\{}", tweak.subkey, tweak.value_name).to_lowercase();
    FENCED_FRAGMENTS
        .iter()
        .find(|fragment| path.contains(*fragment))
        .map(|fragment| format!("`{}` touches a fenced area (`{fragment}`)", tweak.id))
}

/// `HKCU\...\Value`, as the UI and the audit log show it.
fn display_path(tweak: &Tweak) -> String {
    format!(
        r"{}\{}\{}",
        tweak.hive.short_name(),
        tweak.subkey,
        tweak.value_name
    )
}

/// The value as a number, `None` when it is not set, or an error when it holds
/// something other than a `REG_DWORD` — which the catalogue never writes, so
/// it is left alone rather than overwritten.
fn observe(value: Option<&RegValue>) -> std::result::Result<Option<u32>, String> {
    match value {
        None => Ok(None),
        Some(value) => value.as_dword().map(Some).ok_or_else(|| {
            format!(
                "`{}` holds something other than a number; WinKnife leaves it alone",
                value.name
            )
        }),
    }
}

/// One catalogue entry as the machine has it now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TweakState {
    pub id: String,
    pub name: String,
    pub reason: String,
    pub category: TweakCategory,
    /// Which Windows versions it does anything on.
    pub applies_to: String,
    /// Exact registry path, value name included.
    pub path: String,
    /// The value now; `None` when it is not set.
    pub current: Option<u32>,
    /// The value applying writes.
    pub target: u32,
    /// WinKnife set it and it still holds what WinKnife wrote, so undo is
    /// available.
    pub applied: bool,
    pub restart: Restart,
    pub undoability: Undoability,
    pub source_url: String,
    /// Why the value could not be judged; such a row gets no action.
    pub error: Option<String>,
    /// WinKnife set this value, and something changed it since (C7). The
    /// reason says what was expected and what is there now. Re-applying is the
    /// ordinary apply, offered by [`recommendation`], never done on its own.
    pub drift: Option<String>,
}

/// An open undo record.
struct UndoRow {
    id: i64,
    hive: String,
    subkey: String,
    value_name: String,
    prior: Option<(u32, Vec<u8>)>,
    applied: (u32, Vec<u8>),
    applied_at: String,
}

fn open_row(conn: &Connection, tweak_id: &str) -> Result<Option<UndoRow>> {
    Ok(conn
        .query_row(
            "SELECT id, hive, subkey, value_name, prior_kind, prior_bytes,
                    applied_kind, applied_bytes, applied_at
             FROM tweak_undo
             WHERE tweak_id = ?1 AND closed_at IS NULL
             ORDER BY id DESC LIMIT 1",
            params![tweak_id],
            |row| {
                let prior_kind: Option<u32> = row.get(4)?;
                let prior_bytes: Option<Vec<u8>> = row.get(5)?;
                Ok(UndoRow {
                    id: row.get(0)?,
                    hive: row.get(1)?,
                    subkey: row.get(2)?,
                    value_name: row.get(3)?,
                    prior: prior_kind.zip(prior_bytes),
                    applied: (row.get(6)?, row.get(7)?),
                    applied_at: row.get(8)?,
                })
            },
        )
        .optional()?)
}

fn close_row(conn: &Connection, id: i64, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE tweak_undo SET closed_at = ?2, close_reason = ?3 WHERE id = ?1",
        params![id, Utc::now().to_rfc3339(), reason],
    )?;
    Ok(())
}

fn same_value(value: Option<&RegValue>, expected: &(u32, Vec<u8>)) -> bool {
    value.is_some_and(|value| value.kind == expected.0 && value.bytes == expected.1)
}

/// Why a value WinKnife set no longer holds what it wrote, or `None` when it
/// still does. Read-only.
fn drift_reason(tweak: &Tweak, row: &UndoRow, live: Option<&RegValue>) -> Option<String> {
    if same_value(live, &row.applied) {
        return None;
    }
    let found = match live {
        None => "it is not set".to_owned(),
        Some(value) => match value.as_dword() {
            Some(number) => format!("it holds {number}"),
            None => "it holds something other than a number".to_owned(),
        },
    };
    let when = row.applied_at.get(..10).unwrap_or(&row.applied_at);
    Some(format!(
        "WinKnife set this to {} on {when}, but {found} now. Windows Update, another app or a manual change may have reset it.",
        tweak.target
    ))
}

/// The tweak engine: catalogue, fence, apply with undo record, undo.
pub struct TweakService {
    registry: Arc<dyn Registry>,
    catalog: Vec<Tweak>,
}

impl TweakService {
    /// The service over the shipped catalogue.
    pub fn new(registry: Arc<dyn Registry>) -> Self {
        Self {
            registry,
            catalog: CATALOG.to_vec(),
        }
    }

    #[cfg(test)]
    fn with_catalog(registry: Arc<dyn Registry>, catalog: Vec<Tweak>) -> Self {
        Self { registry, catalog }
    }

    /// What is holding Windows Update back, if anything. Read-only (C7).
    pub fn update_status(&self, now: chrono::DateTime<chrono::Utc>) -> update_block::UpdateStatus {
        update_block::check(self.registry.as_ref(), now)
    }

    /// A known tweak inside the fence.
    fn find(&self, tweak_id: &str) -> Result<&Tweak> {
        let tweak = self
            .catalog
            .iter()
            .find(|tweak| tweak.id == tweak_id)
            .ok_or_else(|| CoreError::Tweak(format!("unknown tweak `{tweak_id}`")))?;
        match fence_refusal(tweak) {
            Some(reason) => Err(CoreError::Tweak(reason)),
            None => Ok(tweak),
        }
    }

    fn live(&self, tweak: &Tweak) -> Result<Option<RegValue>> {
        self.registry
            .value(tweak.hive, tweak.subkey, tweak.value_name)
    }

    /// Read every catalogue entry. Read-only.
    ///
    /// # Errors
    /// Returns an error only if the undo table cannot be read.
    pub fn scan(&self, conn: &Connection) -> Result<Vec<TweakState>> {
        self.catalog
            .iter()
            .map(|tweak| {
                let live = self.live(tweak);
                let (current, error) = match &live {
                    Err(err) => (None, Some(err.to_string())),
                    Ok(value) => match observe(value.as_ref()) {
                        Ok(current) => (current, fence_refusal(tweak)),
                        Err(reason) => (None, Some(reason)),
                    },
                };
                let drift = match (open_row(conn, tweak.id)?, &live) {
                    (Some(row), Ok(value)) => Some(drift_reason(tweak, &row, value.as_ref())),
                    _ => None,
                };
                // Applied: on record, and the live value is still what was written.
                let applied = drift == Some(None);
                Ok(TweakState {
                    id: tweak.id.to_owned(),
                    name: tweak.name.to_owned(),
                    reason: tweak.reason.to_owned(),
                    category: tweak.category,
                    applies_to: tweak.applies_to.to_owned(),
                    path: display_path(tweak),
                    current,
                    target: tweak.target,
                    applied,
                    restart: tweak.restart,
                    undoability: tweak.undo,
                    source_url: tweak.source_url.to_owned(),
                    error,
                    drift: drift.flatten(),
                })
            })
            .collect()
    }

    /// Put back what was there before WinKnife's latest write of `tweak_id`.
    ///
    /// # Errors
    /// Returns an error when there is nothing to undo, the value changed since
    /// WinKnife wrote it, or the restore fails or does not read back.
    pub fn undo(&self, conn: &Connection, tweak_id: &str) -> Result<()> {
        let row = open_row(conn, tweak_id)?
            .ok_or_else(|| CoreError::Tweak(format!("nothing to undo for `{tweak_id}`")))?;
        if row.hive != Hive::CurrentUser.short_name() {
            return Err(CoreError::Tweak(format!(
                "undo record for `{tweak_id}` points outside HKCU"
            )));
        }
        let hive = Hive::CurrentUser;
        let live = self.registry.value(hive, &row.subkey, &row.value_name)?;
        if !same_value(live.as_ref(), &row.applied) {
            return Err(CoreError::Tweak(format!(
                "`{}` changed since WinKnife set it; left as it is",
                row.value_name
            )));
        }

        match &row.prior {
            Some((kind, bytes)) => self.registry.set_value(
                hive,
                &row.subkey,
                &RegValue {
                    name: row.value_name.clone(),
                    kind: *kind,
                    bytes: bytes.clone(),
                },
            )?,
            None => self
                .registry
                .delete_value(hive, &row.subkey, &row.value_name)?,
        }

        let after = self.registry.value(hive, &row.subkey, &row.value_name)?;
        let restored = match &row.prior {
            Some(prior) => same_value(after.as_ref(), prior),
            None => after.is_none(),
        };
        if !restored {
            return Err(CoreError::Tweak(format!(
                "`{}` did not read back as it was before",
                row.value_name
            )));
        }
        close_row(conn, row.id, "undone")
    }

    /// Put `prior` back after a write that did not read back.
    fn roll_back(&self, tweak: &Tweak, prior: Option<&RegValue>) -> Result<()> {
        match prior {
            Some(value) => self.registry.set_value(tweak.hive, tweak.subkey, value),
            None => self
                .registry
                .delete_value(tweak.hive, tweak.subkey, tweak.value_name),
        }
    }
}

impl TweakControl for TweakService {
    fn refusal(&self, tweak_id: &str, observed: Option<u32>) -> Option<String> {
        let tweak = match self.find(tweak_id) {
            Ok(tweak) => tweak,
            Err(err) => return Some(format!("skipped: {err}")),
        };
        let path = display_path(tweak);
        let live = match self.live(tweak) {
            Ok(value) => value,
            Err(err) => return Some(format!("skipped: could not read `{path}`: {err}")),
        };
        match observe(live.as_ref()) {
            Err(reason) => Some(format!("skipped: {reason}")),
            Ok(now) if now != observed => Some(format!("skipped: `{path}` changed since the scan")),
            Ok(Some(now)) if now == tweak.target => {
                Some(format!("skipped: `{path}` already holds {now}"))
            }
            Ok(_) => None,
        }
    }

    fn apply(&self, conn: &Connection, tweak_id: &str) -> Result<()> {
        let tweak = self.find(tweak_id)?;
        let prior = self.live(tweak)?;
        if observe(prior.as_ref()).map_err(CoreError::Tweak)? == Some(tweak.target) {
            return Err(CoreError::Tweak(format!(
                "`{}` already holds {}",
                display_path(tweak),
                tweak.target
            )));
        }
        let applied = RegValue::dword(tweak.value_name, tweak.target);

        // The record goes in before the write, so a crash in between still
        // leaves the prior value on file.
        conn.execute(
            "INSERT INTO tweak_undo (tweak_id, hive, subkey, value_name, prior_kind,
                                     prior_bytes, applied_kind, applied_bytes, applied_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                tweak.id,
                tweak.hive.short_name(),
                tweak.subkey,
                tweak.value_name,
                prior.as_ref().map(|value| value.kind),
                prior.as_ref().map(|value| value.bytes.clone()),
                applied.kind,
                applied.bytes,
                Utc::now().to_rfc3339(),
            ],
        )?;
        let row = conn.last_insert_rowid();
        // A write that never landed has nothing to undo, so its record goes.
        let forget = || conn.execute("DELETE FROM tweak_undo WHERE id = ?1", params![row]);

        if let Err(err) = self.registry.set_value(tweak.hive, tweak.subkey, &applied) {
            forget()?;
            return Err(err);
        }

        let after = self.live(tweak)?;
        if after.as_ref() != Some(&applied) {
            // Keep the record open if the prior value cannot be put back, so
            // the mismatch stays visible instead of vanishing.
            self.roll_back(tweak, prior.as_ref())?;
            forget()?;
            return Err(CoreError::Tweak(format!(
                "`{}` did not read back as written; the old value was put back",
                display_path(tweak)
            )));
        }

        conn.execute(
            "UPDATE tweak_undo SET closed_at = ?2, close_reason = 'superseded'
             WHERE tweak_id = ?1 AND closed_at IS NULL AND id <> ?3",
            params![tweak.id, Utc::now().to_rfc3339(), row],
        )?;
        Ok(())
    }
}

/// The finding and recommendation applying `state` goes through, or `None`
/// when it is applied already, unreadable, or at its target.
pub fn recommendation(state: &TweakState) -> Option<(Finding, Recommendation)> {
    if state.applied || state.error.is_some() || state.current == Some(state.target) {
        return None;
    }
    let mut finding = Finding::new("tweaks", PathBuf::from(&state.path), FindingKind::RegistryEntry);
    finding.meta = serde_json::json!({ "tweakId": state.id });
    let recommendation = Recommendation::new(
        vec![finding.id],
        RiskBucket::NeedsReview,
        MAX_CONFIDENCE,
        Action::ApplyTweak,
        state.reason.clone(),
        // A setting frees no disk space.
        0,
        Justification::Tweak {
            tweak_id: state.id.clone(),
            observed: state.current,
        },
    )
    .map_err(|err| tracing::warn!(tweak = state.id.as_str(), %err, "tweak recommendation rejected"))
    .ok()?;
    Some((finding, recommendation))
}

#[derive(Debug, Default)]
pub struct TweaksModule;

impl ToolModule for TweaksModule {
    fn id(&self) -> &'static str {
        "tweaks"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Tweaks".to_owned(),
            description: "Change a few Windows settings, each with an undo that puts back \
                          exactly what was there."
                .to_owned(),
            icon: "settings".to_owned(),
            capabilities: ModuleCapabilities {
                has_scan: true,
                has_execute: true,
                has_settings: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::open_in_memory;
    use crate::core::platform::MapRegistry;

    const EXT: &str = "show-file-extensions";

    fn service(registry: MapRegistry) -> (Arc<MapRegistry>, TweakService) {
        let registry = Arc::new(registry);
        let service = TweakService::new(registry.clone());
        (registry, service)
    }

    fn hide_file_ext(registry: &MapRegistry) -> Option<RegValue> {
        registry
            .value(Hive::CurrentUser, EXPLORER_ADVANCED, "HideFileExt")
            .expect("read")
    }

    fn open_rows(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM tweak_undo WHERE closed_at IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("count")
    }

    #[test]
    fn every_shipped_tweak_is_inside_the_fence() {
        for tweak in CATALOG {
            assert_eq!(fence_refusal(tweak), None, "{}", tweak.id);
            assert_ne!(tweak.undo, Undoability::NotUndoable);
        }
    }

    #[test]
    fn catalogue_is_well_formed() {
        let mut ids = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        for tweak in CATALOG {
            assert!(ids.insert(tweak.id), "duplicate id {}", tweak.id);
            assert!(paths.insert(display_path(tweak)), "duplicate value {}", tweak.id);
            assert_eq!(tweak.hive, Hive::CurrentUser, "{}", tweak.id);
            assert!(tweak.reason.len() > 20, "{} needs a reason", tweak.id);
            assert!(!tweak.name.is_empty() && !tweak.applies_to.is_empty(), "{}", tweak.id);
            assert!(tweak.source_url.starts_with("https://"), "{}", tweak.id);
            assert!(!tweak.subkey.is_empty() && !tweak.value_name.is_empty(), "{}", tweak.id);
            assert!(tweak.target <= 3, "{} target looks wrong", tweak.id);
            let expected = if tweak.restart == Restart::None {
                Undoability::Reversible
            } else {
                Undoability::ReversibleAfterRestart
            };
            assert_eq!(tweak.undo, expected, "{}", tweak.id);
        }
        assert!(CATALOG.len() >= 15);
    }

    #[test]
    fn every_shipped_tweak_round_trips_on_a_test_registry() {
        let conn = open_in_memory().expect("db");
        for (n, tweak) in CATALOG.iter().enumerate() {
            // Alternate between a value that was set and one that was absent.
            let seeded = n % 2 == 0;
            let original = RegValue::dword(tweak.value_name, tweak.target ^ 1);
            let registry = if seeded {
                MapRegistry::new().with_key(tweak.hive, tweak.subkey, vec![original.clone()])
            } else {
                MapRegistry::new()
            };
            let (registry, service) = service(registry);
            service.apply(&conn, tweak.id).expect(tweak.id);
            let live = registry.value(tweak.hive, tweak.subkey, tweak.value_name).expect("read");
            assert_eq!(live.and_then(|v| v.as_dword()), Some(tweak.target), "{}", tweak.id);
            service.undo(&conn, tweak.id).expect(tweak.id);
            let back = registry.value(tweak.hive, tweak.subkey, tweak.value_name).expect("read");
            assert_eq!(back, seeded.then_some(original), "{}", tweak.id);
        }
        assert_eq!(open_rows(&conn), 0);
    }

    #[test]
    fn apply_then_undo_puts_the_original_bytes_back() {
        let original = RegValue::dword("HideFileExt", 1);
        let (registry, service) = service(MapRegistry::new().with_key(
            Hive::CurrentUser,
            EXPLORER_ADVANCED,
            vec![original.clone()],
        ));
        let conn = open_in_memory().expect("db");

        assert_eq!(service.refusal(EXT, Some(1)), None);
        service.apply(&conn, EXT).expect("apply");
        assert_eq!(hide_file_ext(&registry).and_then(|v| v.as_dword()), Some(0));
        assert!(service.scan(&conn).expect("scan")[0].applied);

        service.undo(&conn, EXT).expect("undo");
        assert_eq!(hide_file_ext(&registry), Some(original));
        assert_eq!(open_rows(&conn), 0);
        assert!(!service.scan(&conn).expect("scan")[0].applied);
    }

    #[test]
    fn a_value_that_was_absent_is_absent_again_after_undo() {
        let (registry, service) = service(MapRegistry::new());
        let conn = open_in_memory().expect("db");

        assert_eq!(service.scan(&conn).expect("scan")[0].current, None);
        service.apply(&conn, EXT).expect("apply");
        assert!(hide_file_ext(&registry).is_some());
        service.undo(&conn, EXT).expect("undo");
        assert_eq!(hide_file_ext(&registry), None);
    }

    #[test]
    fn the_fence_refuses_without_writing() {
        let fenced = [
            Tweak {
                id: "hklm",
                hive: Hive::LocalMachine,
                ..CATALOG[0]
            },
            Tweak {
                id: "no-undo",
                undo: Undoability::NotUndoable,
                ..CATALOG[0]
            },
            Tweak {
                id: "start",
                value_name: "Start_TrackDocs",
                ..CATALOG[0]
            },
            Tweak {
                id: "update",
                subkey: r"Software\Microsoft\WindowsUpdate\UX",
                ..CATALOG[0]
            },
        ];
        let registry = Arc::new(MapRegistry::new());
        let service = TweakService::with_catalog(registry.clone(), fenced.to_vec());
        let conn = open_in_memory().expect("db");

        for tweak in &fenced {
            assert!(service.refusal(tweak.id, None).is_some(), "{}", tweak.id);
            assert!(service.apply(&conn, tweak.id).is_err(), "{}", tweak.id);
            assert!(recommendation(&service.scan(&conn).expect("scan")[0]).is_none());
        }
        assert!(registry.writes().is_empty());
        assert_eq!(open_rows(&conn), 0);
    }

    #[test]
    fn refusal_catches_a_changed_or_already_set_value() {
        let (_, service) = service(MapRegistry::new().with_key(
            Hive::CurrentUser,
            EXPLORER_ADVANCED,
            vec![RegValue::dword("HideFileExt", 0)],
        ));
        assert!(service.refusal(EXT, Some(1)).is_some(), "changed since scan");
        assert!(service.refusal(EXT, Some(0)).is_some(), "already the target");
        assert!(service.refusal("nope", None).is_some(), "unknown");
    }

    #[test]
    fn a_non_number_value_is_left_alone() {
        let (registry, service) = service(MapRegistry::new().with_key(
            Hive::CurrentUser,
            EXPLORER_ADVANCED,
            vec![RegValue::string("HideFileExt", 1, "odd")],
        ));
        let conn = open_in_memory().expect("db");
        let state = &service.scan(&conn).expect("scan")[0];
        assert!(state.error.is_some());
        assert!(recommendation(state).is_none());
        assert!(service.refusal(EXT, None).is_some());
        assert!(service.apply(&conn, EXT).is_err());
        assert!(registry.writes().is_empty());
    }

    #[test]
    fn undo_refuses_when_the_value_changed_since() {
        let (registry, service) = service(MapRegistry::new());
        let conn = open_in_memory().expect("db");
        service.apply(&conn, EXT).expect("apply");
        registry
            .set_value(
                Hive::CurrentUser,
                EXPLORER_ADVANCED,
                &RegValue::dword("HideFileExt", 7),
            )
            .expect("user change");

        assert!(service.undo(&conn, EXT).is_err());
        assert_eq!(hide_file_ext(&registry).and_then(|v| v.as_dword()), Some(7));
        assert_eq!(open_rows(&conn), 1);
    }

    #[test]
    fn a_failed_write_leaves_no_undo_record() {
        let (_, service) = service(MapRegistry::new().refusing_writes("access denied"));
        let conn = open_in_memory().expect("db");
        assert!(service.apply(&conn, EXT).is_err());
        assert_eq!(open_rows(&conn), 0);
    }

    /// A registry whose writes report success but never land.
    struct Deaf(MapRegistry);

    impl Registry for Deaf {
        fn values(&self, hive: Hive, subkey: &str) -> Result<Vec<RegValue>> {
            self.0.values(hive, subkey)
        }
        fn value(&self, hive: Hive, subkey: &str, name: &str) -> Result<Option<RegValue>> {
            self.0.value(hive, subkey, name)
        }
        fn set_value(&self, _: Hive, _: &str, _: &RegValue) -> Result<()> {
            Ok(())
        }
        fn delete_value(&self, hive: Hive, subkey: &str, name: &str) -> Result<()> {
            self.0.delete_value(hive, subkey, name)
        }
    }

    #[test]
    fn a_write_that_does_not_read_back_fails_and_keeps_the_old_value() {
        let original = RegValue::dword("HideFileExt", 1);
        let registry = Arc::new(Deaf(MapRegistry::new().with_key(
            Hive::CurrentUser,
            EXPLORER_ADVANCED,
            vec![original.clone()],
        )));
        let service = TweakService::new(registry.clone());
        let conn = open_in_memory().expect("db");

        assert!(service.apply(&conn, EXT).is_err());
        assert_eq!(
            registry
                .value(Hive::CurrentUser, EXPLORER_ADVANCED, "HideFileExt")
                .expect("read"),
            Some(original)
        );
        assert_eq!(open_rows(&conn), 0);
    }

    #[test]
    fn reapplying_supersedes_the_older_record() {
        let (registry, service) = service(MapRegistry::new());
        let conn = open_in_memory().expect("db");
        service.apply(&conn, EXT).expect("first apply");
        registry
            .set_value(
                Hive::CurrentUser,
                EXPLORER_ADVANCED,
                &RegValue::dword("HideFileExt", 1),
            )
            .expect("user change");
        service.apply(&conn, EXT).expect("second apply");
        assert_eq!(open_rows(&conn), 1);

        service.undo(&conn, EXT).expect("undo");
        assert_eq!(hide_file_ext(&registry).and_then(|v| v.as_dword()), Some(1));
    }

    #[test]
    fn scan_offers_only_what_is_not_already_set() {
        let (_, service) = service(MapRegistry::new().with_key(
            Hive::CurrentUser,
            EXPLORER_ADVANCED,
            vec![RegValue::dword("HideFileExt", 0)],
        ));
        let conn = open_in_memory().expect("db");
        let states = service.scan(&conn).expect("scan");
        assert_eq!(states.len(), CATALOG.len());
        assert_eq!(states[0].path, format!(r"HKCU\{EXPLORER_ADVANCED}\HideFileExt"));
        assert!(recommendation(&states[0]).is_none());
        let (_, rec) = recommendation(&states[1]).expect("hidden files offered");
        assert_eq!(rec.action, Action::ApplyTweak);
        assert!(!rec.reason.is_empty());
    }

    #[test]
    fn drift_is_reported_with_a_reason_and_nothing_is_written() {
        let (registry, service) = service(MapRegistry::new());
        let conn = open_in_memory().expect("db");
        service.apply(&conn, EXT).expect("apply");
        assert_eq!(service.scan(&conn).expect("scan")[0].drift, None);

        // Something else puts the old value back.
        registry
            .set_value(
                Hive::CurrentUser,
                EXPLORER_ADVANCED,
                &RegValue::dword("HideFileExt", 1),
            )
            .expect("outside change");
        let writes = registry.writes().len();

        let state = &service.scan(&conn).expect("scan")[0];
        let reason = state.drift.as_deref().expect("drift");
        assert!(reason.contains("set this to 0"), "{reason}");
        assert!(reason.contains("holds 1"), "{reason}");
        assert!(!state.applied);
        assert_eq!(registry.writes().len(), writes, "scan is read-only");

        // Removed altogether reads as "not set".
        registry
            .delete_value(Hive::CurrentUser, EXPLORER_ADVANCED, "HideFileExt")
            .expect("outside delete");
        let state = &service.scan(&conn).expect("scan")[0];
        assert!(state.drift.as_deref().expect("drift").contains("not set"));

        // A tweak WinKnife never set has no drift, whatever its value.
        assert_eq!(service.scan(&conn).expect("scan")[1].drift, None);
    }

    #[test]
    fn reapplying_a_drifted_tweak_goes_through_apply_and_undo_restores_the_drifted_value() {
        let (registry, service) = service(MapRegistry::new());
        let conn = open_in_memory().expect("db");
        service.apply(&conn, EXT).expect("apply");
        registry
            .set_value(
                Hive::CurrentUser,
                EXPLORER_ADVANCED,
                &RegValue::dword("HideFileExt", 1),
            )
            .expect("outside change");

        let state = &service.scan(&conn).expect("scan")[0];
        let (_, rec) = recommendation(state).expect("re-apply offered");
        let Justification::Tweak { observed, .. } = rec.justification else {
            panic!("tweak justification");
        };
        assert_eq!(service.refusal(EXT, observed), None);
        service.apply(&conn, EXT).expect("re-apply");
        let state = &service.scan(&conn).expect("scan")[0];
        assert!(state.applied);
        assert_eq!(state.drift, None);
        assert_eq!(open_rows(&conn), 1);

        service.undo(&conn, EXT).expect("undo");
        assert_eq!(hide_file_ext(&registry).and_then(|v| v.as_dword()), Some(1));
    }
}
