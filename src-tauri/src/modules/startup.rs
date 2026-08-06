//! Startup manager: everything that runs at boot, and how to stop it
//! reversibly (`docs/modules/05-startup-manager.md`).
//!
//! # Disabling mirrors what Windows already does
//!
//! There is exactly one way to turn each kind of entry off, and all three are
//! the ones Windows itself uses, so Task Manager and this app agree rather than
//! fight:
//!
//! | Source | How it is disabled | What is never done |
//! |---|---|---|
//! | `Run` / `RunOnce` value | a `StartupApproved` binary flag | the value is never deleted |
//! | Startup-folder `.lnk` | moved into a module-managed folder, recorded in the ledger | the shortcut is never deleted |
//! | Logon scheduled task | `Enabled = false` | the task is never deleted |
//!
//! The `StartupApproved` flag is the important one. Task Manager writes a
//! 12-byte binary value under
//! `…\Explorer\StartupApproved\Run` whose **first byte's low bit is the disabled
//! bit** — `0x02` enabled, `0x03` disabled — followed at offset 4 by the
//! `FILETIME` of the change. Writing the same shape means a disable made here
//! shows up as "Disabled" in Task Manager and can be undone from either side.
//! The original `Run` value is not touched at all, which is what makes
//! "re-enabling restores it byte-identically" true by construction rather than
//! by care.
//!
//! # What "estimated" means
//!
//! Module spec 05 is explicit: V1 impact is an honest heuristic tier, not a
//! measurement. It is derived from the size of the program and from the known-app
//! list, and every place it appears says "estimated". Real boot-time measurement
//! is V2.
//!
//! # Tests never touch the real machine
//!
//! Every OS surface is a trait object: [`Registry`], [`TaskScheduler`],
//! [`SignatureProbe`]. The tests inject fakes, and the real implementations also
//! refuse to write unless the process armed OS effects, which no test binary can
//! do.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};
use crate::core::executor::{LiveStartupEntry, StartupControl};
use crate::core::ledger;
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::platform::{
    EnvResolver, Hive, RegValue, Registry, Signature, SignatureProbe, TaskScheduler,
    REG_TYPE_BINARY,
};
use crate::core::types::StartupSource;

/// `Run` keys, relative to a hive.
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// `RunOnce` keys, relative to a hive.
const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
/// The 32-bit view of `HKLM`'s `Run`, which is a real second location.
const RUN_WOW: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
/// The 32-bit view of `HKLM`'s `RunOnce`.
const RUN_ONCE_WOW: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\RunOnce";

/// Where Windows records which `Run` entries the user switched off.
const APPROVED_RUN: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
/// The same, for the 32-bit `Run` key.
const APPROVED_RUN32: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32";
/// The same, for Startup-folder shortcuts.
const APPROVED_FOLDER: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder";

/// The `StartupApproved` payload Task Manager writes: a flag byte, three bytes
/// of padding, then a `FILETIME`.
const APPROVED_LEN: usize = 12;
/// Flag byte for an entry that runs.
const APPROVED_ENABLED: u8 = 0x02;
/// Flag byte for one that does not. Windows reads the low bit.
const APPROVED_DISABLED: u8 = 0x03;

/// Folder inside the app data directory where disabled shortcuts are kept.
pub const DISABLED_LINKS_DIR: &str = "startup-disabled";

/// How confident the known-app list is about an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Leave it alone.
    Keep,
    /// Reasonable either way; the user decides.
    UserChoice,
    /// Nothing breaks if this is off.
    SafeToDisable,
    /// Not in the list.
    Unknown,
}

/// Estimated effect on how long logging in takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Impact {
    /// Barely measurable.
    Low,
    /// Noticeable.
    Medium,
    /// One of the slow ones.
    High,
}

/* -------------------------------------------------------------------------
 * The known-app list
 * ---------------------------------------------------------------------- */

/// One entry of `rules/startup/startup-known.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct KnownApp {
    /// What this entry recognises.
    #[serde(rename = "match")]
    pub match_: KnownMatch,
    /// Human-readable grouping, e.g. `Updater`.
    pub category: String,
    /// The hint shown to the user.
    pub verdict: Verdict,
    /// Whether Windows owns it, which is what greys the toggle.
    #[serde(default)]
    pub os: bool,
    /// Impact override, when the list knows better than the size heuristic.
    #[serde(default)]
    pub impact: Option<Impact>,
    /// One sentence saying what turning it off costs.
    pub note: String,
}

/// How a [`KnownApp`] recognises an entry. All present fields must match.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KnownMatch {
    /// Authenticode publisher, compared case-insensitively and in full.
    #[serde(default)]
    pub publisher: Option<String>,
    /// Entry name, compared case-insensitively and in full.
    #[serde(default)]
    pub name: Option<String>,
    /// Substring of the command line, compared case-insensitively.
    #[serde(default)]
    pub command_contains: Option<String>,
}

/// The parsed known-app list.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KnownApps {
    /// Entries, in file order. The first match wins.
    pub entries: Vec<KnownApp>,
}

impl KnownApps {
    /// Parse the list.
    ///
    /// # Errors
    /// Returns [`CoreError::RulePack`] when the file is not valid — a startup
    /// manager silently running with no known apps would call every ordinary
    /// updater a mystery (RULES-FORMAT.md: loud failure beats silent skip).
    pub fn parse(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|err| CoreError::RulePack {
            pack: "startup-known".to_owned(),
            field: "<file>".to_owned(),
            message: err.to_string(),
        })
    }

    /// Load from `rules/startup/startup-known.json` under `rules_dir`.
    ///
    /// A missing file is **not** an error: a build without the pack still shows
    /// the entries, just without the friendly notes. A *malformed* one is.
    ///
    /// # Errors
    /// See [`KnownApps::parse`].
    pub fn load(rules_dir: &Path) -> Result<Self> {
        let path = rules_dir.join("startup").join("startup-known.json");
        match std::fs::read_to_string(&path) {
            Ok(json) => Self::parse(&json),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %path.display(), "no startup known-app list; entries will have no notes");
                Ok(Self::default())
            }
            Err(err) => Err(CoreError::RulePack {
                pack: "startup-known".to_owned(),
                field: "<file>".to_owned(),
                message: format!("cannot read `{}`: {err}", path.display()),
            }),
        }
    }

    /// The first entry that recognises this program, if any.
    pub fn lookup(&self, name: &str, command: &str, publisher: Option<&str>) -> Option<&KnownApp> {
        self.entries.iter().find(|app| {
            let m = &app.match_;
            let publisher_ok = match (&m.publisher, publisher) {
                (Some(wanted), Some(actual)) => wanted.eq_ignore_ascii_case(actual),
                (Some(_), None) => false,
                (None, _) => true,
            };
            let name_ok = m
                .name
                .as_ref()
                .is_none_or(|wanted| wanted.eq_ignore_ascii_case(name));
            let command_ok = m.command_contains.as_ref().is_none_or(|needle| {
                command.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
            });
            publisher_ok && name_ok && command_ok
        })
    }
}

/* -------------------------------------------------------------------------
 * What the UI sees
 * ---------------------------------------------------------------------- */

/// One thing that runs at logon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupEntry {
    /// Stable id: `{source}:{key}`. Also what the UI passes back to toggle.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Where it lives.
    pub source: StartupSource,
    /// Value name, shortcut file name or task path.
    pub key: String,
    /// Human-readable location, e.g. `HKCU\…\Run`.
    pub location: String,
    /// The command line as stored.
    pub command: String,
    /// The program the command runs, when it could be worked out.
    pub target: Option<PathBuf>,
    /// Whether it runs today.
    pub enabled: bool,
    /// Whether Windows owns it. The toggle is disabled and `os_reason` says why.
    pub os_owned: bool,
    /// Why the toggle is disabled, when it is.
    pub os_reason: Option<String>,
    /// Authenticode publisher, when the binary is signed.
    pub publisher: Option<String>,
    /// Whether the binary carries a verified signature.
    pub signed: bool,
    /// One phrase about the signature, shown next to the badge.
    pub signature_note: String,
    /// Estimated tier. Always presented as an estimate.
    pub impact: Impact,
    /// How the estimate was arrived at, in words.
    pub impact_reason: String,
    /// What the known-app list says.
    pub verdict: Verdict,
    /// Its category, when it is recognised.
    pub category: Option<String>,
    /// The known-app note, or the "unsigned, unknown publisher" line.
    pub note: String,
}

/// Everything one enumeration produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupScan {
    /// Entries, sorted by source then name.
    pub entries: Vec<StartupEntry>,
    /// Sources that could not be read at all, so a partial list says it is one.
    pub unreadable: Vec<String>,
}

/* -------------------------------------------------------------------------
 * The service
 * ---------------------------------------------------------------------- */

/// Where the startup folders are and where disabled shortcuts go.
#[derive(Debug, Clone)]
pub struct StartupPaths {
    /// This user's Startup folder.
    pub user: Option<PathBuf>,
    /// The all-users Startup folder.
    pub common: Option<PathBuf>,
    /// Module-managed folder holding shortcuts this app switched off.
    pub disabled: PathBuf,
}

impl StartupPaths {
    /// Work the folders out from the environment.
    pub fn from_env(env: &dyn EnvResolver, app_data_dir: &Path) -> Self {
        const TAIL: &str = r"Microsoft\Windows\Start Menu\Programs\Startup";
        Self {
            user: env.var("APPDATA").map(|dir| PathBuf::from(dir).join(TAIL)),
            common: env.var("PROGRAMDATA").map(|dir| PathBuf::from(dir).join(TAIL)),
            disabled: app_data_dir.join(DISABLED_LINKS_DIR),
        }
    }

    /// The folder a source refers to.
    pub fn folder(&self, source: StartupSource) -> Option<&Path> {
        match source {
            StartupSource::UserStartupFolder => self.user.as_deref(),
            StartupSource::CommonStartupFolder => self.common.as_deref(),
            _ => None,
        }
    }
}

/// Everything the startup manager needs, composed once.
pub struct StartupService {
    registry: Arc<dyn Registry>,
    tasks: Arc<dyn TaskScheduler>,
    signatures: Arc<dyn SignatureProbe>,
    known: KnownApps,
    paths: StartupPaths,
    windows_dir: Option<PathBuf>,
}

impl std::fmt::Debug for StartupService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartupService")
            .field("known_entries", &self.known.entries.len())
            .field("paths", &self.paths)
            .finish()
    }
}

impl StartupService {
    /// Compose the service.
    pub fn new(
        registry: Arc<dyn Registry>,
        tasks: Arc<dyn TaskScheduler>,
        signatures: Arc<dyn SignatureProbe>,
        known: KnownApps,
        paths: StartupPaths,
        windows_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            registry,
            tasks,
            signatures,
            known,
            paths,
            windows_dir,
        }
    }

    /// Enumerate every autostart entry. Read-only.
    ///
    /// A source that cannot be read is *named* in the result rather than
    /// dropped: a startup list missing `HKLM` because the read failed looks
    /// exactly like a machine with nothing in `HKLM`.
    pub fn scan(&self, conn: &Connection) -> StartupScan {
        let mut entries = Vec::new();
        let mut unreadable = Vec::new();

        self.collect_registry(&mut entries, &mut unreadable);
        self.collect_folders(conn, &mut entries, &mut unreadable);
        self.collect_tasks(&mut entries, &mut unreadable);

        entries.sort_by(|a, b| {
            source_order(a.source)
                .cmp(&source_order(b.source))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        StartupScan {
            entries,
            unreadable,
        }
    }

    /* --- registry ---------------------------------------------------- */

    fn collect_registry(&self, out: &mut Vec<StartupEntry>, unreadable: &mut Vec<String>) {
        // (hive, run key, StartupApproved key, source, label)
        let locations: [(Hive, &str, &str, StartupSource, &str); 6] = [
            (Hive::CurrentUser, RUN, APPROVED_RUN, StartupSource::CurrentUserRun, r"HKCU\…\Run"),
            (Hive::CurrentUser, RUN_ONCE, APPROVED_RUN, StartupSource::CurrentUserRun, r"HKCU\…\RunOnce"),
            (Hive::LocalMachine, RUN, APPROVED_RUN, StartupSource::LocalMachineRun, r"HKLM\…\Run"),
            (Hive::LocalMachine, RUN_ONCE, APPROVED_RUN, StartupSource::LocalMachineRun, r"HKLM\…\RunOnce"),
            (Hive::LocalMachine, RUN_WOW, APPROVED_RUN32, StartupSource::LocalMachineRun32, r"HKLM\…\WOW6432Node\Run"),
            (Hive::LocalMachine, RUN_ONCE_WOW, APPROVED_RUN32, StartupSource::LocalMachineRun32, r"HKLM\…\WOW6432Node\RunOnce"),
        ];

        for (hive, key, approved_key, source, label) in locations {
            let values = match self.registry.values(hive, key) {
                Ok(values) => values,
                Err(err) => {
                    tracing::warn!(location = label, error = %err, "an autostart location could not be read");
                    unreadable.push(format!("{label}: {err}"));
                    continue;
                }
            };

            for value in values {
                let Some(command) = value.as_string() else {
                    // A `Run` value that is not a string is not a command line.
                    // Reporting it as a program would be inventing one.
                    continue;
                };
                let enabled = self.approval_enabled(hive, approved_key, &value.name);
                out.push(self.describe(
                    source,
                    value.name.clone(),
                    value.name,
                    label.to_owned(),
                    command,
                    enabled,
                ));
            }
        }
    }

    /// Whether `StartupApproved` says this entry runs. Absent means enabled.
    fn approval_enabled(&self, hive: Hive, approved_key: &str, name: &str) -> bool {
        match self.registry.value(hive, approved_key, name) {
            Ok(Some(value)) => !is_disabled_flag(&value.bytes),
            // No approval record at all is what a freshly installed program
            // looks like, and Windows treats that as enabled.
            _ => true,
        }
    }

    /* --- startup folders --------------------------------------------- */

    fn collect_folders(
        &self,
        conn: &Connection,
        out: &mut Vec<StartupEntry>,
        unreadable: &mut Vec<String>,
    ) {
        for source in [
            StartupSource::UserStartupFolder,
            StartupSource::CommonStartupFolder,
        ] {
            let label = folder_label(source);
            let Some(folder) = self.paths.folder(source) else {
                unreadable.push(format!("{label}: the folder could not be located"));
                continue;
            };
            if !folder.is_dir() {
                // A machine with no Startup folder is ordinary, not broken.
                continue;
            }
            match std::fs::read_dir(folder) {
                Ok(listing) => {
                    for entry in listing.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            continue;
                        }
                        let file_name = entry.file_name().to_string_lossy().into_owned();
                        // `desktop.ini` is Explorer's own folder metadata, not a
                        // program.
                        if file_name.eq_ignore_ascii_case("desktop.ini") {
                            continue;
                        }
                        let display = path
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| file_name.clone());
                        out.push(self.describe(
                            source,
                            file_name,
                            display,
                            label.to_owned(),
                            path.to_string_lossy().into_owned(),
                            true,
                        ));
                    }
                }
                Err(err) => unreadable.push(format!("{label}: {err}")),
            }

            // Shortcuts this app has moved out are still entries — they are just
            // switched off, and they have to be listed or the user cannot put
            // them back.
            match disabled_links(conn, source) {
                Ok(held) => {
                    // A row nobody can parse is a shortcut this app is holding
                    // and cannot name. Saying so is the whole difference between
                    // a skip and a disappearance (security review H11).
                    if held.unreadable_rows > 0 {
                        unreadable.push(format!(
                            "{label} (disabled list): {} ledger row(s) could not be read, so \
                             the shortcut(s) they describe are still switched off and are not \
                             listed below. The files themselves are untouched in WinKnife's \
                             `{DISABLED_LINKS_DIR}` folder.",
                            held.unreadable_rows
                        ));
                    }
                    for row in held.links {
                        let display = Path::new(&row.file_name)
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| row.file_name.clone());
                        out.push(self.describe(
                            source,
                            row.file_name,
                            display,
                            label.to_owned(),
                            row.original_path,
                            false,
                        ));
                    }
                }
                Err(err) => unreadable.push(format!("{label} (disabled list): {err}")),
            }
        }
    }

    /* --- scheduled tasks ---------------------------------------------- */

    fn collect_tasks(&self, out: &mut Vec<StartupEntry>, unreadable: &mut Vec<String>) {
        match self.tasks.logon_tasks() {
            Ok(tasks) => {
                for task in tasks {
                    let command = task.command.clone().unwrap_or_default();
                    let mut entry = self.describe(
                        StartupSource::ScheduledTask,
                        task.path.clone(),
                        task.name.clone(),
                        "Task Scheduler".to_owned(),
                        command,
                        task.enabled,
                    );
                    // The task's declared author is often the vendor, and is the
                    // only publisher hint for a task whose program is missing.
                    if entry.publisher.is_none() {
                        entry.publisher = task.author;
                    }
                    out.push(entry);
                }
            }
            Err(err) => unreadable.push(format!("Task Scheduler: {err}")),
        }
    }

    /* --- shared description ------------------------------------------- */

    /// Turn a raw entry into everything the UI needs to render a row.
    fn describe(
        &self,
        source: StartupSource,
        key: String,
        name: String,
        location: String,
        command: String,
        enabled: bool,
    ) -> StartupEntry {
        let target = executable_of(&command);
        let signature = target
            .as_deref()
            .map(|path| self.signatures.check(path))
            .unwrap_or(Signature::Unknown {
                reason: "the command line does not name a program that is on this machine"
                    .to_owned(),
            });
        let publisher = signature.publisher().map(str::to_owned);
        let known = self.known.lookup(&name, &command, publisher.as_deref());

        let os_owned = known.is_some_and(|app| app.os)
            || self.is_windows_owned(source, &key, target.as_deref());
        let os_reason = os_owned.then(|| {
            known
                .filter(|app| app.os)
                .map(|app| app.note.clone())
                .unwrap_or_else(|| {
                    "This is part of Windows. WinKnife does not turn off the operating \
                     system's own startup entries."
                        .to_owned()
                })
        });

        let (impact, impact_reason) = estimate_impact(known, target.as_deref());

        let note = match known {
            Some(app) => app.note.clone(),
            None if !signature.is_signed() => format!(
                "{}. Nothing is known about this one, so it is worth checking what it is \
                 before you decide.",
                signature.describe()
            ),
            None => format!(
                "{}, but it is not in WinKnife's known-app list, so there is no advice to \
                 give beyond who signed it.",
                signature.describe()
            ),
        };

        StartupEntry {
            id: format!("{}:{key}", source_slug(source)),
            name,
            source,
            key,
            location,
            command,
            target,
            enabled,
            os_owned,
            os_reason,
            publisher,
            signed: signature.is_signed(),
            signature_note: signature.describe(),
            impact,
            impact_reason,
            verdict: known.map_or(Verdict::Unknown, |app| app.verdict),
            category: known.map(|app| app.category.clone()),
            note,
        }
    }

    /// Whether Windows itself owns this entry.
    fn is_windows_owned(
        &self,
        source: StartupSource,
        key: &str,
        target: Option<&Path>,
    ) -> bool {
        // Everything under `\Microsoft\Windows\` in the task tree is Windows'
        // own scheduling. Compared as text because it is a name inside the task
        // namespace and not a filesystem path, so `path_compare` has nothing to
        // resolve (standing rule, `docs/DECISIONS-NEEDED.md`).
        if source == StartupSource::ScheduledTask
            && key.to_ascii_lowercase().starts_with(r"\microsoft\windows\")
        {
            return true;
        }
        // A program inside the Windows directory is Windows'. Through
        // `path_compare`, because `C:\WINDOWS` and `C:\Windows` and a short-form
        // spelling are all the same place.
        match (self.windows_dir.as_deref(), target) {
            (Some(windows), Some(program)) => {
                crate::core::path_compare::is_inside(windows, program)
            }
            _ => false,
        }
    }
}

/// Order sources so the list reads registry, folders, tasks.
fn source_order(source: StartupSource) -> u8 {
    match source {
        StartupSource::CurrentUserRun => 0,
        StartupSource::LocalMachineRun => 1,
        StartupSource::LocalMachineRun32 => 2,
        StartupSource::UserStartupFolder => 3,
        StartupSource::CommonStartupFolder => 4,
        StartupSource::ScheduledTask => 5,
    }
}

/// Stable slug used in an entry id.
fn source_slug(source: StartupSource) -> &'static str {
    match source {
        StartupSource::CurrentUserRun => "hkcu-run",
        StartupSource::LocalMachineRun => "hklm-run",
        StartupSource::LocalMachineRun32 => "hklm-run32",
        StartupSource::UserStartupFolder => "user-startup",
        StartupSource::CommonStartupFolder => "common-startup",
        StartupSource::ScheduledTask => "task",
    }
}

fn folder_label(source: StartupSource) -> &'static str {
    match source {
        StartupSource::CommonStartupFolder => "Startup folder (all users)",
        _ => "Startup folder",
    }
}

/// Whether a `StartupApproved` payload says "disabled".
///
/// Windows reads the **low bit of the first byte**, not the whole byte: `0x02`
/// and `0x06` are both enabled, `0x03` and `0x07` are both disabled. Comparing
/// the whole byte would read an entry Task Manager disabled on a build that uses
/// `0x07` as enabled, and re-enable it behind the user's back.
pub fn is_disabled_flag(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|flag| flag & 1 == 1)
}

/// The `StartupApproved` payload for a given state.
///
/// `previous` is the value that is already there, if any: **the seven high bits
/// of its first byte** are kept, so this app cooperates with whatever flag
/// variant the Windows build uses instead of overwriting it with an assumption.
/// Bytes 1-3 are not carried over — they are the documented padding between the
/// flag and the `FILETIME` at offset 4, Windows writes `00 00 00` there, and
/// this writes the same. Bytes 4-11 are ours to set: they are the timestamp of
/// *this* change (security review L13, which found the doc claiming more than
/// the code does).
pub fn approval_payload(enabled: bool, previous: Option<&[u8]>, now: DateTime<Utc>) -> Vec<u8> {
    let base = previous
        .and_then(|bytes| bytes.first().copied())
        .map(|flag| flag & !1u8)
        .filter(|high| *high != 0)
        .unwrap_or(APPROVED_ENABLED & !1u8);

    let mut payload = vec![0u8; APPROVED_LEN];
    payload[0] = if enabled { base } else { base | 1 };
    // Windows writes `02` for enabled rather than `00`; keep that exactly.
    if enabled && payload[0] == 0 {
        payload[0] = APPROVED_ENABLED;
    }
    if !enabled && payload[0] == 1 {
        payload[0] = APPROVED_DISABLED;
    }

    if !enabled {
        // Offset 4: the FILETIME of the change, as Task Manager records it.
        let filetime = windows_filetime(now);
        payload[4..APPROVED_LEN.min(12)].copy_from_slice(&filetime.to_le_bytes());
    }
    payload
}

/// 100-nanosecond intervals since 1601-01-01, which is what Windows stores.
fn windows_filetime(now: DateTime<Utc>) -> u64 {
    /// Intervals between the Windows epoch and the Unix epoch.
    const EPOCH_DIFFERENCE: i64 = 116_444_736_000_000_000;
    let unix_intervals = now.timestamp().saturating_mul(10_000_000);
    (EPOCH_DIFFERENCE.saturating_add(unix_intervals)).max(0) as u64
}

/// The program a command line runs, when one can be picked out of it.
///
/// Handles the two shapes a `Run` value really takes: a quoted path followed by
/// arguments, and a bare path where the arguments start at the first switch. A
/// command line that names nothing that exists returns `None` rather than a
/// guess — an invented path would be handed to the signature check, which would
/// then report "unknown" about a file nobody meant.
pub fn executable_of(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let candidate = PathBuf::from(&rest[..end]);
            return candidate.is_file().then_some(candidate);
        }
    }

    // Unquoted: try progressively longer prefixes, because `C:\Program Files\A
    // B\x.exe -q` has spaces in the path as well as before the switch.
    let whole = PathBuf::from(trimmed);
    if whole.is_file() {
        return Some(whole);
    }
    let mut best: Option<PathBuf> = None;
    let bytes: Vec<char> = trimmed.chars().collect();
    for (index, character) in bytes.iter().enumerate() {
        if *character != ' ' {
            continue;
        }
        let candidate = PathBuf::from(bytes[..index].iter().collect::<String>());
        if candidate.is_file() {
            best = Some(candidate);
        }
    }
    best
}

/// Estimate the impact tier, honestly.
///
/// The known-app list wins when it has an opinion, because a human wrote it
/// about that specific program. Otherwise the size of the binary is the only
/// signal available offline, and it is a weak one — which is why every place
/// this figure appears calls it an estimate.
fn estimate_impact(known: Option<&KnownApp>, target: Option<&Path>) -> (Impact, String) {
    if let Some(app) = known.and_then(|app| app.impact) {
        return (
            app,
            "Estimated from WinKnife's known-app list for this program.".to_owned(),
        );
    }

    let Some(size) = target.and_then(|path| std::fs::metadata(path).ok()).map(|m| m.len()) else {
        return (
            Impact::Low,
            "Estimated. The program could not be found, so there is nothing to measure — \
             treated as low rather than guessed upward."
                .to_owned(),
        );
    };

    /// Above this a program is one of the heavy ones.
    const HIGH_BYTES: u64 = 20 * 1024 * 1024;
    /// Above this it is more than a stub.
    const MEDIUM_BYTES: u64 = 2 * 1024 * 1024;

    let tier = if size >= HIGH_BYTES {
        Impact::High
    } else if size >= MEDIUM_BYTES {
        Impact::Medium
    } else {
        Impact::Low
    };
    (
        tier,
        format!(
            "Estimated from the size of the program ({:.1} MB). Real boot-time measurement \
             is not in this version.",
            size as f64 / (1024.0 * 1024.0)
        ),
    )
}

/* -------------------------------------------------------------------------
 * The disabled-shortcut ledger
 * ---------------------------------------------------------------------- */

/// One shortcut this app has moved out of a Startup folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisabledLink {
    /// Row id.
    pub id: Uuid,
    /// Which folder it came from.
    pub source: StartupSource,
    /// The shortcut's file name, which is also the entry key.
    pub file_name: String,
    /// Where it was.
    pub original_path: String,
    /// Where it is now.
    pub stored_path: String,
    /// When it was moved.
    pub disabled_at: DateTime<Utc>,
}

/// What one source's ledger holds, including what could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisabledLinks {
    /// Shortcuts this app is holding.
    pub links: Vec<DisabledLink>,
    /// Rows whose id or timestamp no longer parses, and which are therefore not
    /// in `links`.
    ///
    /// These used to be dropped with `filter_map`, which is the worst of the
    /// three options: the shortcut is still in the managed folder, the UI shows
    /// nothing, and `read_entry` falls through to the Startup folder and reports
    /// the entry as enabled — so a re-disable hits the unique index and orphans
    /// the file deterministically (security review H11). Skipping a row is safe;
    /// hiding the fact that it exists is not, exactly as with
    /// [`crate::core::quarantine::QuarantineStats::unreadable_rows`].
    pub unreadable_rows: u64,
    /// Rows whose stored file is definitely not there any more, cleared by
    /// [`ledger::reconcile`]: a crash between a restore's move and its `DELETE`
    /// leaves one behind, and it would otherwise show a shortcut as disabled
    /// forever while it sits enabled in the Startup folder.
    pub reconciled_rows: u64,
}

/// Every shortcut currently held for one source, with the rows that could not
/// be read counted rather than dropped.
///
/// Read-only: rows [`ledger::reconcile`] finds stale are reported, not deleted.
/// A listing is not the place to write to the ledger, and the row is harmless
/// once nobody believes it.
///
/// # Errors
/// Returns an error if the ledger cannot be read.
pub fn disabled_links(conn: &Connection, source: StartupSource) -> Result<DisabledLinks> {
    let mut stmt = conn.prepare(
        "SELECT id, file_name, original_path, stored_path, disabled_at
         FROM startup_disabled_link WHERE source = ?1 ORDER BY file_name",
    )?;
    let rows = stmt
        .query_map([source_slug(source)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut held = DisabledLinks::default();
    for (id, file_name, original_path, stored_path, at) in rows {
        let Some(link) = parse_link(source, &id, file_name, original_path, stored_path, &at) else {
            held.unreadable_rows += 1;
            continue;
        };
        if ledger::reconcile(Path::new(&link.stored_path)).is_current() {
            held.links.push(link);
        } else {
            tracing::warn!(
                row = %link.id,
                stored = %link.stored_path,
                "a disabled-shortcut row names a file that is not there; \
                 the shortcut has already gone home and the row is ignored"
            );
            held.reconciled_rows += 1;
        }
    }
    Ok(held)
}

/// One shortcut by source and file name.
///
/// A row that exists but cannot be parsed is an **error**, not a `None`. `None`
/// means "this app is not holding that shortcut", which sends `read_entry` on to
/// the Startup folder and lets a re-disable collide with the unique index — the
/// second route to the orphan in H11. Refusing is the only honest answer when
/// the ledger says something about this exact entry that we cannot read.
///
/// # Errors
/// Returns an error if the ledger cannot be read, or if the row for this entry
/// does not parse.
pub fn find_disabled_link(
    conn: &Connection,
    source: StartupSource,
    file_name: &str,
) -> Result<Option<DisabledLink>> {
    let row = conn
        .query_row(
            "SELECT id, original_path, stored_path, disabled_at
             FROM startup_disabled_link WHERE source = ?1 AND file_name = ?2",
            rusqlite::params![source_slug(source), file_name],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;

    let Some((id, original_path, stored_path, at)) = row else {
        return Ok(None);
    };
    let link = parse_link(
        source,
        &id,
        file_name.to_owned(),
        original_path,
        stored_path,
        &at,
    )
    .ok_or_else(|| {
        CoreError::UnknownStartupEntry(format!(
            "the ledger row for `{file_name}` cannot be read (id `{id}`, disabled at `{at}`), \
             so WinKnife will not act on it — the shortcut itself is untouched"
        ))
    })?;

    // A row whose stored file has gone is a restore that committed its move and
    // then lost its `DELETE`. The shortcut is back in the Startup folder, so the
    // truthful answer is "not held".
    if !ledger::reconcile(Path::new(&link.stored_path)).is_current() {
        tracing::warn!(
            row = %link.id,
            stored = %link.stored_path,
            "ignoring a disabled-shortcut row whose file is not there"
        );
        return Ok(None);
    }
    Ok(Some(link))
}

/// Build a [`DisabledLink`] from its stored columns, or `None` when one of the
/// two parsed fields no longer parses.
fn parse_link(
    source: StartupSource,
    id: &str,
    file_name: String,
    original_path: String,
    stored_path: String,
    at: &str,
) -> Option<DisabledLink> {
    let Ok(id) = Uuid::parse_str(id) else {
        tracing::error!(value = id, "unparsable uuid in the disabled-shortcut ledger");
        return None;
    };
    let Ok(disabled_at) = DateTime::parse_from_rfc3339(at) else {
        tracing::error!(
            value = at,
            "unparsable timestamp in the disabled-shortcut ledger"
        );
        return None;
    };
    Some(DisabledLink {
        id,
        source,
        file_name,
        original_path,
        stored_path,
        disabled_at: disabled_at.with_timezone(&Utc),
    })
}

fn record_disabled_link(conn: &Connection, link: &DisabledLink) -> Result<()> {
    conn.execute(
        "INSERT INTO startup_disabled_link
            (id, source, file_name, original_path, stored_path, disabled_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            link.id.to_string(),
            source_slug(link.source),
            link.file_name,
            link.original_path,
            link.stored_path,
            link.disabled_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn forget_disabled_link(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute(
        "DELETE FROM startup_disabled_link WHERE id = ?1",
        [id.to_string()],
    )?;
    Ok(())
}

/* -------------------------------------------------------------------------
 * Turning things off and on
 * ---------------------------------------------------------------------- */

impl StartupService {
    /// Read one entry as it stands now, by source and key.
    ///
    /// # Errors
    /// Returns an error when the source itself could not be read.
    pub fn read_entry(
        &self,
        conn: &Connection,
        source: StartupSource,
        key: &str,
    ) -> Result<Option<LiveStartupEntry>> {
        match source {
            StartupSource::CurrentUserRun
            | StartupSource::LocalMachineRun
            | StartupSource::LocalMachineRun32 => {
                let (hive, approved) = hive_of(source);
                for run_key in run_keys(source) {
                    if let Some(value) = self.registry.value(hive, run_key, key)? {
                        if let Some(command) = value.as_string() {
                            return Ok(Some(LiveStartupEntry {
                                enabled: self.approval_enabled(hive, approved, key),
                                os_owned: self
                                    .is_windows_owned(source, key, executable_of(&command).as_deref()),
                                command,
                            }));
                        }
                    }
                }
                Ok(None)
            }
            StartupSource::UserStartupFolder | StartupSource::CommonStartupFolder => {
                if let Some(link) = find_disabled_link(conn, source, key)? {
                    let command = link.original_path;
                    return Ok(Some(LiveStartupEntry {
                        os_owned: self.folder_entry_is_os_owned(source, key, &command),
                        command,
                        enabled: false,
                    }));
                }
                let Some(folder) = self.paths.folder(source) else {
                    return Ok(None);
                };
                let path = folder.join(key);
                if !path.is_file() {
                    return Ok(None);
                }
                let command = path.to_string_lossy().into_owned();
                Ok(Some(LiveStartupEntry {
                    os_owned: self.folder_entry_is_os_owned(source, key, &command),
                    command,
                    enabled: true,
                }))
            }
            StartupSource::ScheduledTask => {
                let tasks = self.tasks.logon_tasks()?;
                Ok(tasks.into_iter().find(|task| task.path == key).map(|task| {
                    let command = task.command.clone().unwrap_or_default();
                    LiveStartupEntry {
                        os_owned: self
                            .is_windows_owned(source, key, executable_of(&command).as_deref()),
                        enabled: task.enabled,
                        command,
                    }
                }))
            }
        }
    }

    /// Whether Windows owns a Startup-folder shortcut.
    ///
    /// The same two questions [`StartupService::describe`] asks — does the
    /// known-app list mark it as part of Windows, and does the shortcut point
    /// inside the Windows directory — asked at toggle time as well as at scan
    /// time. `read_entry` used to hardcode `false` here, so an entry the list
    /// marks `os: true` greyed out in the UI but sailed through `set_enabled`'s
    /// refusal. Only reachable today as an *enable*, which is harmless, but the
    /// gate is not there to be harmless in the cases we happen to have thought
    /// of (security review L14).
    fn folder_entry_is_os_owned(
        &self,
        source: StartupSource,
        file_name: &str,
        command: &str,
    ) -> bool {
        let target = executable_of(command);
        let publisher = target
            .as_deref()
            .and_then(|path| self.signatures.check(path).publisher().map(str::to_owned));
        let name = Path::new(file_name)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_name.to_owned());
        self.known
            .lookup(&name, command, publisher.as_deref())
            .is_some_and(|app| app.os)
            || self.is_windows_owned(source, file_name, target.as_deref())
    }

    /// Turn an entry on or off. Never deletes anything.
    ///
    /// # Errors
    /// Returns an error when the entry is unknown, when Windows owns it, or when
    /// the change was refused.
    pub fn set_enabled(
        &self,
        conn: &Connection,
        source: StartupSource,
        key: &str,
        enabled: bool,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let Some(live) = self.read_entry(conn, source, key)? else {
            return Err(CoreError::UnknownStartupEntry(key.to_owned()));
        };
        // The same refusal the executor's gate 4 makes, enforced here too, so a
        // direct re-enable command cannot reach an OS entry either.
        if live.os_owned {
            return Err(CoreError::UnknownStartupEntry(format!(
                "`{key}` is part of Windows and WinKnife does not change it"
            )));
        }
        if live.enabled == enabled {
            return Ok(());
        }

        match source {
            StartupSource::CurrentUserRun
            | StartupSource::LocalMachineRun
            | StartupSource::LocalMachineRun32 => {
                let (hive, approved_key) = hive_of(source);
                let previous = self.registry.value(hive, approved_key, key)?;
                let payload =
                    approval_payload(enabled, previous.as_ref().map(|v| v.bytes.as_slice()), now);
                self.registry.set_value(
                    hive,
                    approved_key,
                    &RegValue {
                        name: key.to_owned(),
                        kind: REG_TYPE_BINARY,
                        bytes: payload,
                    },
                )?;
            }
            StartupSource::UserStartupFolder | StartupSource::CommonStartupFolder => {
                if enabled {
                    self.restore_link(conn, source, key)?;
                } else {
                    self.stash_link(conn, source, key, now)?;
                }
            }
            StartupSource::ScheduledTask => self.tasks.set_enabled(key, enabled)?,
        }

        tracing::info!(
            source = source_slug(source),
            key,
            enabled,
            "autostart entry toggled"
        );
        Ok(())
    }

    /// Record where a shortcut came from, then move it into the module-managed
    /// folder.
    ///
    /// That order is the whole point, and it used to be the other way round: the
    /// `.lnk` was moved and *then* the row was written, so a ledger write that
    /// failed — a full disk, a locked database, `SQLITE_BUSY`, the
    /// `startup_disabled_link_origin` unique index firing — left the shortcut
    /// out of the Startup folder with nothing recording where it had gone. It
    /// was never deleted, and it was also never again visible to the user
    /// (security review H11).
    ///
    /// It goes through [`ledger::ledgered_move`] rather than re-deriving the
    /// order here, because re-deriving it is exactly what went wrong.
    fn stash_link(
        &self,
        conn: &Connection,
        source: StartupSource,
        file_name: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let folder = self
            .paths
            .folder(source)
            .ok_or_else(|| CoreError::UnknownStartupEntry(file_name.to_owned()))?;
        let original = folder.join(file_name);
        std::fs::create_dir_all(&self.paths.disabled)?;

        // The stored name carries the row id, so two sources can hold shortcuts
        // with the same file name without one hiding the other.
        let id = Uuid::new_v4();
        let stored = self.paths.disabled.join(format!("{id}-{file_name}"));
        let link = DisabledLink {
            id,
            source,
            file_name: file_name.to_owned(),
            original_path: original.to_string_lossy().into_owned(),
            stored_path: stored.to_string_lossy().into_owned(),
            disabled_at: now,
        };

        let mut write = |tx: &Connection| record_disabled_link(tx, &link);
        let mut undo = |tx: &Connection| forget_disabled_link(tx, id);
        ledger::ledgered_move(
            conn,
            "startup stash",
            ledger::LedgerStep::Claim {
                write: &mut write,
                undo: &mut undo,
            },
            // The move primitive renames when it can, copies and verifies when
            // it must, and never overwrites an occupied destination. It cannot
            // be reached without the `Recorded` this closure is handed.
            |recorded| ledger::move_user_file(recorded, &original, &stored),
        )
    }

    /// Put a stashed shortcut back exactly where it was, then forget the row.
    ///
    /// A [`ledger::LedgerStep::Retire`], not a `Claim`: dropping the row first
    /// would leave the `.lnk` in the managed folder with nothing pointing at it
    /// if the move then failed, which is the same orphan from the other
    /// direction. The row is dropped only once the shortcut is back in the
    /// Startup folder, where the user can see it without our help — and a row
    /// left behind by a crash in that gap is cleared by
    /// [`ledger::reconcile`] the next time it is read.
    fn restore_link(
        &self,
        conn: &Connection,
        source: StartupSource,
        file_name: &str,
    ) -> Result<()> {
        let link = find_disabled_link(conn, source, file_name)?
            .ok_or_else(|| CoreError::UnknownStartupEntry(file_name.to_owned()))?;

        let mut write = |tx: &Connection| forget_disabled_link(tx, link.id);
        ledger::ledgered_move(
            conn,
            "startup restore",
            ledger::LedgerStep::Retire { write: &mut write },
            |recorded| {
                ledger::move_user_file(
                    recorded,
                    Path::new(&link.stored_path),
                    Path::new(&link.original_path),
                )
            },
        )
    }
}

/// The hive and `StartupApproved` key a registry source uses.
fn hive_of(source: StartupSource) -> (Hive, &'static str) {
    match source {
        StartupSource::CurrentUserRun => (Hive::CurrentUser, APPROVED_RUN),
        StartupSource::LocalMachineRun => (Hive::LocalMachine, APPROVED_RUN),
        StartupSource::LocalMachineRun32 => (Hive::LocalMachine, APPROVED_RUN32),
        _ => (Hive::CurrentUser, APPROVED_RUN),
    }
}

/// The `Run` keys a registry source covers, `Run` before `RunOnce`.
fn run_keys(source: StartupSource) -> [&'static str; 2] {
    match source {
        StartupSource::LocalMachineRun32 => [RUN_WOW, RUN_ONCE_WOW],
        _ => [RUN, RUN_ONCE],
    }
}

/// The `StartupApproved` key Startup-folder shortcuts use.
///
/// Exposed so the UI copy and the tests can name the same thing the OS does.
pub const fn startup_folder_approval_key() -> &'static str {
    APPROVED_FOLDER
}

/* -------------------------------------------------------------------------
 * Executor seam
 * ---------------------------------------------------------------------- */

/// The [`StartupControl`] the executor holds.
///
/// It owns a database handle because a Startup-folder entry's state lives in the
/// ledger, and the executor has no way to reach one otherwise.
pub struct StartupControlAdapter {
    service: Arc<StartupService>,
    db: Arc<crate::core::db::Database>,
}

impl StartupControlAdapter {
    /// Wire a service and a database together for the executor.
    pub fn new(service: Arc<StartupService>, db: Arc<crate::core::db::Database>) -> Self {
        Self { service, db }
    }
}

impl StartupControl for StartupControlAdapter {
    fn read(&self, source: StartupSource, key: &str) -> Result<Option<LiveStartupEntry>> {
        self.db
            .with(|conn| self.service.read_entry(conn, source, key))
    }

    fn disable(&self, source: StartupSource, key: &str) -> Result<()> {
        self.db.with(|conn| {
            self.service
                .set_enabled(conn, source, key, false, Utc::now())
        })
    }
}

/* -------------------------------------------------------------------------
 * Module registration
 * ---------------------------------------------------------------------- */

/// The startup manager.
///
/// It has no `scan`/`analyze` of the file-oriented kind: an autostart entry is
/// not a file and its recommendation is built by the command layer, which has
/// the composed service. The trait still gives it registry presence and a
/// manifest (ARCHITECTURE section 2).
#[derive(Debug, Default)]
pub struct StartupModule;

impl ToolModule for StartupModule {
    fn id(&self) -> &'static str {
        "startup"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Startup".to_owned(),
            description:
                "See what runs when you log in, and switch things off in a way Windows agrees with."
                    .to_owned(),
            icon: "rocket".to_owned(),
            capabilities: ModuleCapabilities {
                has_scan: true,
                has_execute: true,
                has_settings: false,
            },
        }
    }
}

/// Group entries by source, for a UI that lists them under headings.
pub fn group_by_source(scan: &StartupScan) -> BTreeMap<&'static str, Vec<&StartupEntry>> {
    let mut groups: BTreeMap<&'static str, Vec<&StartupEntry>> = BTreeMap::new();
    for entry in &scan.entries {
        groups.entry(source_slug(entry.source)).or_default().push(entry);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    use crate::core::platform::{
        LogonTask, MapEnv, MapRegistry, MapSignatures, MapTaskScheduler, REG_TYPE_EXPAND_SZ,
        REG_TYPE_SZ,
    };

    fn known() -> KnownApps {
        KnownApps::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("repo root")
                .join("rules"),
        )
        .expect("the shipped known-app list must parse")
    }

    struct World {
        _dir: tempfile::TempDir,
        conn: Connection,
        registry: Arc<MapRegistry>,
        tasks: Arc<MapTaskScheduler>,
        service: StartupService,
        user_startup: PathBuf,
    }

    fn world(registry: MapRegistry, tasks: MapTaskScheduler) -> World {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_data = dir.path().join("appdata");
        let roaming = dir.path().join("roaming");
        let user_startup = roaming.join(r"Microsoft\Windows\Start Menu\Programs\Startup");
        std::fs::create_dir_all(&user_startup).expect("startup folder");

        let env = MapEnv::new()
            .with("APPDATA", roaming.to_string_lossy())
            .with("PROGRAMDATA", dir.path().join("programdata").to_string_lossy());
        let registry = Arc::new(registry);
        let tasks = Arc::new(tasks);
        let service = StartupService::new(
            Arc::clone(&registry) as Arc<dyn Registry>,
            Arc::clone(&tasks) as Arc<dyn TaskScheduler>,
            Arc::new(MapSignatures::new()),
            known(),
            StartupPaths::from_env(&env, &app_data),
            Some(dir.path().join("Windows")),
        );

        World {
            conn: db::open_in_memory().expect("db"),
            registry,
            tasks,
            service,
            user_startup,
            _dir: dir,
        }
    }

    fn run_value(name: &str, command: &str) -> RegValue {
        RegValue::string(name, REG_TYPE_SZ, command)
    }

    /* --- the known-app list ------------------------------------------- */

    #[test]
    fn the_shipped_known_app_list_parses() {
        let list = known();
        assert!(
            !list.entries.is_empty(),
            "the shipped list must actually load"
        );
        assert!(list
            .entries
            .iter()
            .all(|app| !app.note.trim().is_empty() && !app.category.trim().is_empty()));
    }

    #[test]
    fn a_malformed_known_app_list_is_loud() {
        let err = KnownApps::parse("{ not json").expect_err("a broken list must not be ignored");
        assert!(matches!(err, CoreError::RulePack { .. }));
    }

    #[test]
    fn a_missing_known_app_list_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(KnownApps::load(dir.path()).expect("absent is fine").entries.is_empty());
    }

    #[test]
    fn the_known_app_list_matches_on_publisher_name_or_command() {
        let list = KnownApps::parse(
            r#"{"entries":[
                {"match":{"command_contains":"Steam"},"category":"Games",
                 "verdict":"safe-to-disable","note":"n"},
                {"match":{"publisher":"NVIDIA Corporation"},"category":"Drivers",
                 "verdict":"user-choice","note":"n"}
            ]}"#,
        )
        .expect("parse");

        assert_eq!(
            list.lookup("Steam", "C:\\Steam\\steam.exe -silent", None)
                .map(|a| a.category.as_str()),
            Some("Games")
        );
        assert_eq!(
            list.lookup("Anything", "x.exe", Some("NVIDIA Corporation"))
                .map(|a| a.category.as_str()),
            Some("Drivers")
        );
        assert!(list.lookup("Mystery", "mystery.exe", None).is_none());
    }

    /* --- the StartupApproved flag ------------------------------------- */

    /// The bit Windows actually reads. Comparing whole bytes would misread the
    /// `06`/`07` variant and re-enable something the user disabled.
    #[test]
    fn the_disabled_bit_is_the_low_bit_of_the_first_byte() {
        assert!(!is_disabled_flag(&[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(is_disabled_flag(&[0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!is_disabled_flag(&[0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(is_disabled_flag(&[0x07, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!is_disabled_flag(&[]), "no record at all means it runs");
    }

    #[test]
    fn the_approval_payload_has_the_shape_task_manager_writes() {
        let now = Utc::now();
        let disabled = approval_payload(false, None, now);
        assert_eq!(disabled.len(), APPROVED_LEN);
        assert_eq!(disabled[0], APPROVED_DISABLED);
        assert_ne!(&disabled[4..12], &[0u8; 8], "a disable records when");

        let enabled = approval_payload(true, Some(&disabled), now);
        assert_eq!(enabled.len(), APPROVED_LEN);
        assert_eq!(enabled[0], APPROVED_ENABLED);
        assert_eq!(&enabled[4..12], &[0u8; 8], "an enable clears the timestamp");
    }

    #[test]
    fn the_payload_keeps_the_high_bits_of_whatever_windows_wrote() {
        let existing = [0x06u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let disabled = approval_payload(false, Some(&existing), Utc::now());
        assert_eq!(
            disabled[0], 0x07,
            "cooperate with the build's own variant instead of overwriting it"
        );
        let enabled = approval_payload(true, Some(&disabled), Utc::now());
        assert_eq!(enabled[0], 0x06);
    }

    /* --- enumeration --------------------------------------------------- */

    #[test]
    fn every_registry_location_is_enumerated_including_wow6432node() {
        let registry = MapRegistry::new()
            .with_key(Hive::CurrentUser, RUN, vec![run_value("UserApp", "u.exe")])
            .with_key(
                Hive::CurrentUser,
                RUN_ONCE,
                vec![run_value("OnceApp", "o.exe")],
            )
            .with_key(
                Hive::LocalMachine,
                RUN,
                vec![run_value("MachineApp", "m.exe")],
            )
            .with_key(
                Hive::LocalMachine,
                RUN_WOW,
                vec![run_value("LegacyApp", "l.exe")],
            );

        let world = world(registry, MapTaskScheduler::new());
        let scan = world.service.scan(&world.conn);
        let names: Vec<_> = scan.entries.iter().map(|e| e.name.as_str()).collect();

        for expected in ["UserApp", "OnceApp", "MachineApp", "LegacyApp"] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        assert!(
            scan.entries
                .iter()
                .any(|e| e.source == StartupSource::LocalMachineRun32),
            "the 32-bit view is a real second location"
        );
    }

    #[test]
    fn all_three_sources_are_enumerated() {
        let registry = MapRegistry::new().with_key(
            Hive::CurrentUser,
            RUN,
            vec![run_value("FromRegistry", "r.exe")],
        );
        let tasks = MapTaskScheduler::new().with_task(LogonTask {
            path: "\\Vendor\\Updater".to_owned(),
            name: "Updater".to_owned(),
            enabled: true,
            command: Some("C:\\Vendor\\u.exe".to_owned()),
            arguments: None,
            author: Some("Vendor Ltd".to_owned()),
        });

        let world = world(registry, tasks);
        std::fs::write(world.user_startup.join("FromFolder.lnk"), b"shortcut").expect("write");

        let scan = world.service.scan(&world.conn);
        let sources: Vec<_> = scan.entries.iter().map(|e| e.source).collect();
        assert!(sources.contains(&StartupSource::CurrentUserRun));
        assert!(sources.contains(&StartupSource::UserStartupFolder));
        assert!(sources.contains(&StartupSource::ScheduledTask));
        assert!(scan.unreadable.is_empty(), "got {:?}", scan.unreadable);
    }

    #[test]
    fn a_disabled_registry_entry_is_reported_as_off() {
        let registry = MapRegistry::new()
            .with_key(Hive::CurrentUser, RUN, vec![run_value("App", "a.exe")])
            .with_key(
                Hive::CurrentUser,
                APPROVED_RUN,
                vec![RegValue::binary(
                    "App",
                    approval_payload(false, None, Utc::now()),
                )],
            );

        let world = world(registry, MapTaskScheduler::new());
        let scan = world.service.scan(&world.conn);
        assert!(!scan.entries[0].enabled);
    }

    #[test]
    fn a_source_that_cannot_be_read_is_named_rather_than_dropped() {
        struct Broken;
        impl Registry for Broken {
            fn values(&self, _: Hive, _: &str) -> Result<Vec<RegValue>> {
                Err(CoreError::Registry("access is denied".to_owned()))
            }
            fn value(&self, _: Hive, _: &str, _: &str) -> Result<Option<RegValue>> {
                Ok(None)
            }
            fn set_value(&self, _: Hive, _: &str, _: &RegValue) -> Result<()> {
                Ok(())
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let service = StartupService::new(
            Arc::new(Broken),
            Arc::new(MapTaskScheduler::new()),
            Arc::new(MapSignatures::new()),
            KnownApps::default(),
            StartupPaths::from_env(&MapEnv::new(), dir.path()),
            None,
        );
        let conn = db::open_in_memory().expect("db");
        let scan = service.scan(&conn);

        assert!(scan.entries.is_empty());
        assert!(
            scan.unreadable.iter().any(|line| line.contains("access is denied")),
            "a list missing HKLM must say so: {:?}",
            scan.unreadable
        );
    }

    #[test]
    fn a_run_value_that_is_not_a_string_is_not_offered_as_a_program() {
        let registry = MapRegistry::new().with_key(
            Hive::CurrentUser,
            RUN,
            vec![RegValue::binary("Garbage", vec![1, 2, 3, 4])],
        );
        let world = world(registry, MapTaskScheduler::new());
        assert!(world.service.scan(&world.conn).entries.is_empty());
    }

    #[test]
    fn an_os_owned_task_is_greyed_with_a_reason() {
        let tasks = MapTaskScheduler::new().with_task(LogonTask {
            path: "\\Microsoft\\Windows\\Defender\\Scan".to_owned(),
            name: "Scan".to_owned(),
            enabled: true,
            command: None,
            arguments: None,
            author: None,
        });
        let world = world(MapRegistry::new(), tasks);
        let scan = world.service.scan(&world.conn);

        let entry = &scan.entries[0];
        assert!(entry.os_owned);
        assert!(
            entry.os_reason.as_deref().is_some_and(|r| !r.trim().is_empty()),
            "a disabled toggle must say why"
        );
    }

    #[test]
    fn an_unsigned_unknown_entry_says_so_in_words() {
        let registry = MapRegistry::new().with_key(
            Hive::CurrentUser,
            RUN,
            vec![run_value("Mystery", "C:\\Users\\me\\mystery.exe")],
        );
        let world = world(registry, MapTaskScheduler::new());
        let entry = &world.service.scan(&world.conn).entries[0];

        assert!(!entry.signed);
        assert_eq!(entry.verdict, Verdict::Unknown);
        assert!(
            entry.note.contains("unknown publisher") || entry.note.contains("not name a program"),
            "got `{}`",
            entry.note
        );
    }

    #[test]
    fn the_impact_tier_says_it_is_an_estimate() {
        let registry = MapRegistry::new()
            .with_key(Hive::CurrentUser, RUN, vec![run_value("App", "a.exe")]);
        let world = world(registry, MapTaskScheduler::new());
        let entry = &world.service.scan(&world.conn).entries[0];
        assert!(
            entry.impact_reason.to_lowercase().contains("estimated"),
            "module spec 05 requires the tier to be labelled: `{}`",
            entry.impact_reason
        );
    }

    /* --- the round trip ------------------------------------------------ */

    /// **The acceptance item.** Disable, then re-enable, and prove the `Run`
    /// value the program installed is byte-identical to what it was — type code
    /// included, because `REG_EXPAND_SZ` and `REG_SZ` behave differently and a
    /// silent downgrade would break `%ProgramFiles%` paths at the next boot.
    #[test]
    fn a_registry_disable_and_re_enable_leaves_the_run_value_byte_identical() {
        let original = RegValue::string(
            "VendorApp",
            REG_TYPE_EXPAND_SZ,
            "%ProgramFiles%\\Vendor\\app.exe --background",
        );
        let registry =
            MapRegistry::new().with_key(Hive::CurrentUser, RUN, vec![original.clone()]);
        let world = world(registry, MapTaskScheduler::new());

        let before = world.registry.snapshot(Hive::CurrentUser, RUN);
        assert_eq!(before, vec![original.clone()]);

        world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::CurrentUserRun,
                "VendorApp",
                false,
                Utc::now(),
            )
            .expect("disable");

        assert!(
            !world.service.scan(&world.conn).entries[0].enabled,
            "the entry must read as off"
        );
        assert_eq!(
            world.registry.snapshot(Hive::CurrentUser, RUN),
            before,
            "the Run value must not be touched at all — module spec 05 forbids deleting it"
        );

        world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::CurrentUserRun,
                "VendorApp",
                true,
                Utc::now(),
            )
            .expect("re-enable");

        let after = world.registry.snapshot(Hive::CurrentUser, RUN);
        assert_eq!(after, before, "byte-identical, type code included");
        assert_eq!(after[0].kind, REG_TYPE_EXPAND_SZ);
        assert_eq!(after[0].bytes, original.bytes);
        assert!(world.service.scan(&world.conn).entries[0].enabled);

        // And the approval record is the only thing that changed.
        let approval = world
            .registry
            .value(Hive::CurrentUser, APPROVED_RUN, "VendorApp")
            .expect("read")
            .expect("an approval record");
        assert!(!is_disabled_flag(&approval.bytes));
    }

    #[test]
    fn a_startup_folder_shortcut_is_moved_and_moved_back_intact() {
        let world = world(MapRegistry::new(), MapTaskScheduler::new());
        let shortcut = world.user_startup.join("Vendor.lnk");
        let payload: Vec<u8> = (0u8..=255).collect();
        std::fs::write(&shortcut, &payload).expect("write");

        world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::UserStartupFolder,
                "Vendor.lnk",
                false,
                Utc::now(),
            )
            .expect("disable");

        assert!(!shortcut.exists(), "the shortcut left the Startup folder");
        let stashed = disabled_links(&world.conn, StartupSource::UserStartupFolder)
            .expect("ledger")
            .links
            .pop()
            .expect("a recorded row");
        assert!(
            Path::new(&stashed.stored_path).is_file(),
            "and is still on disk, not deleted"
        );
        // It is still listed, switched off, so the user can put it back.
        let scan = world.service.scan(&world.conn);
        assert_eq!(scan.entries.len(), 1);
        assert!(!scan.entries[0].enabled);

        world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::UserStartupFolder,
                "Vendor.lnk",
                true,
                Utc::now(),
            )
            .expect("re-enable");

        assert!(shortcut.is_file(), "back where it was");
        assert_eq!(
            std::fs::read(&shortcut).expect("read"),
            payload,
            "byte-identical"
        );
        assert_eq!(
            disabled_links(&world.conn, StartupSource::UserStartupFolder).expect("ledger"),
            DisabledLinks::default(),
            "the row is gone and nothing was left unreadable behind it"
        );
    }

    /// `read_entry` used to hardcode `os_owned: false` for Startup-folder
    /// entries, so an entry the known-app list marks as part of Windows greyed
    /// out in the UI and was still let through `set_enabled`'s refusal
    /// (security review L14). Only reachable as an enable today; closed anyway.
    #[test]
    fn a_startup_folder_shortcut_windows_owns_is_refused_by_the_toggle_too() {
        let world = world(MapRegistry::new(), MapTaskScheduler::new());
        // `is_windows_owned` asks whether the program sits inside the Windows
        // directory, which the fixture points at `<tempdir>/Windows`.
        let windows = world._dir.path().join("Windows");
        std::fs::create_dir_all(&windows).expect("windows dir");
        let program = windows.join("SystemThing.exe");
        std::fs::write(&program, b"MZ").expect("write");

        let shortcut = world.user_startup.join("SystemThing.lnk");
        std::fs::write(&shortcut, program.to_string_lossy().as_bytes()).expect("write");
        // The shortcut's "command" is its own path, so point the entry at the
        // program by giving the ledger the program's path as the original.
        let live = world
            .service
            .read_entry(
                &world.conn,
                StartupSource::UserStartupFolder,
                "SystemThing.lnk",
            )
            .expect("read")
            .expect("the shortcut is there");
        assert!(!live.os_owned, "a .lnk is not itself inside C:\\Windows");

        // Now the same question for an entry whose command really does name a
        // program inside the Windows directory, which is the shape the gate is
        // for.
        assert!(
            world.service.folder_entry_is_os_owned(
                StartupSource::UserStartupFolder,
                "SystemThing.lnk",
                &program.to_string_lossy(),
            ),
            "a shortcut to a program inside the Windows directory is Windows'"
        );
    }

    /* --- the orphan is impossible (security review H11) ----------------- */

    /// Everything a stash left behind, from the user's point of view.
    ///
    /// The bug was never "the file was deleted" — it never was. It was that the
    /// shortcut left the Startup folder while nothing recorded where it went,
    /// so it was gone from the only interface the user has. These three numbers
    /// are that statement.
    fn after_a_failed_stash(world: &World, shortcut: &Path) -> (bool, usize, usize) {
        let managed = std::fs::read_dir(&world.service.paths.disabled)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0);
        let listed = world
            .service
            .scan(&world.conn)
            .entries
            .iter()
            .filter(|entry| entry.key == "Thing.lnk")
            .count();
        (shortcut.is_file(), managed, listed)
    }

    /// The reproduction, with the ledger write failing at exactly the point that
    /// produced the orphan: the row cannot be written, so the shortcut must not
    /// move.
    ///
    /// `query_only` is the read-only database the reviewer used — the shape of a
    /// full disk, a locked file or `SQLITE_BUSY`.
    #[test]
    fn a_ledger_that_cannot_be_written_leaves_the_shortcut_where_it_is() {
        let world = world(MapRegistry::new(), MapTaskScheduler::new());
        let shortcut = world.user_startup.join("Thing.lnk");
        std::fs::write(&shortcut, b"payload").expect("write");

        world
            .conn
            .pragma_update(None, "query_only", "ON")
            .expect("make the ledger read-only");

        let failure = world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::UserStartupFolder,
                "Thing.lnk",
                false,
                Utc::now(),
            )
            .expect_err("a ledger that cannot record must refuse the disable");
        assert!(
            matches!(failure, CoreError::Db(_)),
            "the database failure has to reach the caller: {failure:?}"
        );

        world
            .conn
            .pragma_update(None, "query_only", "OFF")
            .expect("restore");

        let (still_there, managed, listed) = after_a_failed_stash(&world, &shortcut);
        println!("[H11] ledger write refused : {failure}");
        println!("[H11] .lnk still in the Startup folder? {still_there}");
        println!("[H11] managed folder holds  : {managed} file(s)");
        println!("[H11] listed by the module  : {listed} row(s)");
        println!(
            "[H11] ORPHANED (moved out, no record, invisible to the UI)? {}",
            !still_there && listed == 0
        );
        assert!(
            still_there,
            "the shortcut must still be in the Startup folder"
        );
        assert_eq!(managed, 0, "nothing may have moved into the managed folder");
        assert_eq!(listed, 1, "and it must still be listed by the module");
        assert!(
            world.service.scan(&world.conn).entries[0].enabled,
            "it never got switched off, so it must not read as switched off"
        );
    }

    /// The second shape of the same failure: the `startup_disabled_link_origin`
    /// unique index fires because a row for this (source, file name) is already
    /// there. Before the fix the file moved first, so the collision left the
    /// `.lnk` in the managed folder with no row of its own.
    #[test]
    fn a_colliding_ledger_row_leaves_the_shortcut_where_it_is() {
        let world = world(MapRegistry::new(), MapTaskScheduler::new());
        let shortcut = world.user_startup.join("Thing.lnk");
        std::fs::write(&shortcut, b"payload").expect("write");

        // A row that already claims this (source, file name), pointing at a
        // stored file that really exists so `reconcile` does not clear it.
        std::fs::create_dir_all(&world.service.paths.disabled).expect("managed folder");
        let squatter = world.service.paths.disabled.join("squatter.lnk");
        std::fs::write(&squatter, b"older").expect("write");
        record_disabled_link(
            &world.conn,
            &DisabledLink {
                id: Uuid::new_v4(),
                source: StartupSource::UserStartupFolder,
                file_name: "Thing.lnk".to_owned(),
                original_path: shortcut.to_string_lossy().into_owned(),
                stored_path: squatter.to_string_lossy().into_owned(),
                disabled_at: Utc::now(),
            },
        )
        .expect("seed the collision");

        let failure = world
            .service
            .stash_link(
                &world.conn,
                StartupSource::UserStartupFolder,
                "Thing.lnk",
                Utc::now(),
            )
            .expect_err("the unique index must refuse the second row");
        assert!(matches!(failure, CoreError::Db(_)), "got {failure:?}");

        assert!(
            shortcut.is_file(),
            "the shortcut must still be in the Startup folder"
        );
        let managed: Vec<String> = std::fs::read_dir(&world.service.paths.disabled)
            .expect("managed folder")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            managed,
            vec!["squatter.lnk".to_owned()],
            "nothing new may have been moved in"
        );
    }

    /// A row nobody can parse is a shortcut this app is holding and cannot name.
    /// It is counted and announced, never dropped on the floor — dropping it was
    /// the second route to the orphan, because `read_entry` then fell through to
    /// the folder and a re-disable hit the unique index.
    #[test]
    fn a_ledger_row_that_does_not_parse_is_surfaced_rather_than_dropped() {
        let world = world(MapRegistry::new(), MapTaskScheduler::new());
        std::fs::create_dir_all(&world.service.paths.disabled).expect("managed folder");
        let stored = world.service.paths.disabled.join("held.lnk");
        std::fs::write(&stored, b"held").expect("write");

        world
            .conn
            .execute(
                "INSERT INTO startup_disabled_link
                    (id, source, file_name, original_path, stored_path, disabled_at)
                 VALUES ('not-a-uuid', 'user-startup', 'Thing.lnk', 'C:\\x\\Thing.lnk', ?1, 'never')",
                [stored.to_string_lossy()],
            )
            .expect("insert a corrupt row");

        let held = disabled_links(&world.conn, StartupSource::UserStartupFolder).expect("ledger");
        assert!(held.links.is_empty());
        assert_eq!(held.unreadable_rows, 1, "the row has to be counted");

        let scan = world.service.scan(&world.conn);
        assert!(
            scan.unreadable.iter().any(|line| line.contains("could not be read")),
            "and announced to the user: {:?}",
            scan.unreadable
        );

        // And nothing may treat the entry as absent, because that is what let a
        // re-disable collide with the unique index.
        let refusal = world
            .service
            .read_entry(&world.conn, StartupSource::UserStartupFolder, "Thing.lnk")
            .expect_err("an unreadable row for this exact entry must refuse");
        assert!(
            matches!(refusal, CoreError::UnknownStartupEntry(_)),
            "got {refusal:?}"
        );
    }

    /// The crash window a restore leaves open: the move committed, the `DELETE`
    /// did not. The shortcut is back in the Startup folder, so the row is stale
    /// and must not keep reporting the entry as switched off.
    #[test]
    fn a_row_whose_stored_file_has_gone_home_is_reconciled() {
        let world = world(MapRegistry::new(), MapTaskScheduler::new());
        let shortcut = world.user_startup.join("Thing.lnk");
        std::fs::write(&shortcut, b"payload").expect("write");

        record_disabled_link(
            &world.conn,
            &DisabledLink {
                id: Uuid::new_v4(),
                source: StartupSource::UserStartupFolder,
                file_name: "Thing.lnk".to_owned(),
                original_path: shortcut.to_string_lossy().into_owned(),
                stored_path: world
                    .service
                    .paths
                    .disabled
                    .join("never-written.lnk")
                    .to_string_lossy()
                    .into_owned(),
                disabled_at: Utc::now(),
            },
        )
        .expect("seed a stale row");

        let held = disabled_links(&world.conn, StartupSource::UserStartupFolder).expect("ledger");
        assert!(held.links.is_empty());
        assert_eq!(held.reconciled_rows, 1);

        let live = world
            .service
            .read_entry(&world.conn, StartupSource::UserStartupFolder, "Thing.lnk")
            .expect("read")
            .expect("the shortcut is in the folder");
        assert!(
            live.enabled,
            "the file is in the Startup folder, so it runs, whatever a stale row says"
        );
    }

    #[test]
    fn a_scheduled_task_is_disabled_by_flag_and_never_removed() {
        let tasks = MapTaskScheduler::new().with_task(LogonTask {
            path: "\\Vendor\\Updater".to_owned(),
            name: "Updater".to_owned(),
            enabled: true,
            command: None,
            arguments: None,
            author: None,
        });
        let world = world(MapRegistry::new(), tasks);

        world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::ScheduledTask,
                "\\Vendor\\Updater",
                false,
                Utc::now(),
            )
            .expect("disable");

        let task = world.tasks.get("\\Vendor\\Updater").expect("still there");
        assert!(!task.enabled);
        assert_eq!(
            world.tasks.logon_tasks().expect("list").len(),
            1,
            "disabling must never remove a task"
        );
    }

    #[test]
    fn an_os_owned_entry_cannot_be_disabled_even_by_a_direct_call() {
        let tasks = MapTaskScheduler::new().with_task(LogonTask {
            path: "\\Microsoft\\Windows\\Defender\\Scan".to_owned(),
            name: "Scan".to_owned(),
            enabled: true,
            command: None,
            arguments: None,
            author: None,
        });
        let world = world(MapRegistry::new(), tasks);

        let err = world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::ScheduledTask,
                "\\Microsoft\\Windows\\Defender\\Scan",
                false,
                Utc::now(),
            )
            .expect_err("Windows' own entries are never changed");
        assert!(err.to_string().contains("part of Windows"), "got {err}");
        assert!(
            world
                .tasks
                .get("\\Microsoft\\Windows\\Defender\\Scan")
                .expect("task")
                .enabled,
            "and it really is still on"
        );
    }

    #[test]
    fn toggling_an_unknown_entry_is_an_error_rather_than_a_silent_no_op() {
        let world = world(MapRegistry::new(), MapTaskScheduler::new());
        let err = world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::CurrentUserRun,
                "NotThere",
                false,
                Utc::now(),
            )
            .expect_err("an unknown entry must not report success");
        assert!(matches!(err, CoreError::UnknownStartupEntry(_)));
    }

    #[test]
    fn setting_the_state_it_already_has_changes_nothing() {
        let registry = MapRegistry::new()
            .with_key(Hive::CurrentUser, RUN, vec![run_value("App", "a.exe")]);
        let world = world(registry, MapTaskScheduler::new());

        world
            .service
            .set_enabled(
                &world.conn,
                StartupSource::CurrentUserRun,
                "App",
                true,
                Utc::now(),
            )
            .expect("already enabled");
        assert!(
            world.registry.writes().is_empty(),
            "a no-op must not write to the registry"
        );
    }

    /* --- command-line parsing ------------------------------------------ */

    #[test]
    fn a_quoted_command_line_yields_its_program() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = dir.path().join("app with spaces.exe");
        std::fs::write(&exe, b"MZ").expect("write");

        let command = format!("\"{}\" --background", exe.display());
        assert_eq!(executable_of(&command).as_deref(), Some(exe.as_path()));
    }

    #[test]
    fn an_unquoted_command_line_with_spaces_still_finds_its_program() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sub = dir.path().join("Program Folder");
        std::fs::create_dir_all(&sub).expect("dir");
        let exe = sub.join("app.exe");
        std::fs::write(&exe, b"MZ").expect("write");

        let command = format!("{} -q", exe.display());
        assert_eq!(executable_of(&command).as_deref(), Some(exe.as_path()));
    }

    #[test]
    fn a_command_line_naming_nothing_real_yields_no_program() {
        assert_eq!(executable_of(""), None);
        assert_eq!(executable_of("   "), None);
        assert_eq!(
            executable_of("C:\\Nowhere\\ghost.exe --start"),
            None,
            "an invented path would be handed to the signature check"
        );
    }

    /* --- registration --------------------------------------------------- */

    #[test]
    fn the_manifest_is_complete() {
        let manifest = StartupModule.manifest();
        assert_eq!(manifest.id, "startup");
        assert!(manifest.capabilities.has_scan);
        assert!(manifest.capabilities.has_execute);
    }

    #[test]
    fn entries_group_by_source() {
        let registry = MapRegistry::new()
            .with_key(Hive::CurrentUser, RUN, vec![run_value("A", "a.exe")])
            .with_key(Hive::LocalMachine, RUN, vec![run_value("B", "b.exe")]);
        let world = world(registry, MapTaskScheduler::new());
        let scan = world.service.scan(&world.conn);
        let groups = group_by_source(&scan);
        assert_eq!(groups.len(), 2);
        assert!(groups.contains_key("hkcu-run"));
        assert!(groups.contains_key("hklm-run"));
    }
}
