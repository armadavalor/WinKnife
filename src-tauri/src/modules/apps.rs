//! Installed apps (C19): a read-only "what is installed and who ships it" list.
//!
//! Win32 programs come from the three `Uninstall` keys, Store (Appx) packages
//! from the per-user package repository in `HKCU` (no PowerShell, no
//! `Get-AppxPackage`). Nothing here can remove, repair or change an app: the
//! only actions are "open Windows' own Apps settings" and "show the install
//! folder". The Authenticode signer is looked up lazily, per row, through the
//! existing [`SignatureProbe`], so the list itself loads from the registry only.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::platform::{Hive, PanelLauncher, RegValue, Registry, Signature, SignatureProbe};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_WOW: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
const APPX_PACKAGES: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
/// The page Windows itself uses for removing apps; opened through the shell.
const SETTINGS_URI: &str = "ms-settings:appsfeatures";

/// Where an entry was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AppSource {
    /// A classic installer's `Uninstall` entry.
    Win32,
    /// A Store / Appx package.
    Store,
}

/// What kind of package an entry is. Win32 entries are always `App`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageKind {
    /// A normal app.
    App,
    /// A shared runtime other apps depend on (VCLibs, .NET Native, UI.Xaml...).
    Framework,
    /// A language or scale resource pack.
    Resource,
}

/// One row on the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEntry {
    /// Stable within a list; the only thing the UI sends back.
    pub id: String,
    /// Win32 or Store.
    pub source: AppSource,
    /// Shown name; a Store package whose name is unresolved shows its family.
    pub name: String,
    /// Publisher as the installer registered it.
    pub publisher: Option<String>,
    /// Version as the installer registered it.
    pub version: Option<String>,
    /// `YYYY-MM-DD` when the installer wrote a valid date.
    pub install_date: Option<String>,
    /// Estimated size in KiB, as registered (Win32 only).
    pub size_kb: Option<u64>,
    /// Folder to reveal in Explorer.
    pub install_folder: Option<String>,
    /// Package family (`Name_PublisherId`) for Store entries.
    pub family: Option<String>,
    /// App, framework or resource pack.
    pub kind: PackageKind,
    /// Whether a signer lookup is possible (a resolvable exe).
    pub can_check_signer: bool,
}

/// The list plus anything that could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsList {
    /// Win32 entries first, then Store, each sorted by name.
    pub entries: Vec<AppEntry>,
    /// Registry locations that could not be read, for the page to mention.
    pub notes: Vec<String>,
}

/// Outcome of one signer lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignerState {
    /// Verified signature.
    Signed,
    /// No valid signature: "needs review".
    Unsigned,
    /// The check could not be answered.
    Unknown,
}

/// What `apps_signer` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignerInfo {
    /// Signed, unsigned or unknown.
    pub state: SignerState,
    /// Certificate subject when signed and readable.
    pub publisher: Option<String>,
}

type Row = (AppEntry, Option<PathBuf>);

fn text(values: &[RegValue], name: &str) -> Option<String> {
    let found = values.iter().find(|v| v.name.eq_ignore_ascii_case(name))?;
    let trimmed = found.as_string()?.trim().to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn dword(values: &[RegValue], name: &str) -> Option<u32> {
    values
        .iter()
        .find(|v| v.name.eq_ignore_ascii_case(name))?
        .as_dword()
}

/// A folder path from a registry string: unquoted, no trailing slash, and
/// never one that still needs `%VAR%` expansion.
fn clean_folder(raw: &str) -> Option<String> {
    let path = raw.trim().trim_matches('"').trim_end_matches(['\\', '/']);
    (!path.is_empty() && !path.contains('%')).then(|| path.to_owned())
}

/// The exe a `DisplayIcon` points at (`"C:\a\b.exe",0` -> `C:\a\b.exe`).
/// `.ico` and `.dll` icons are not programs, so they resolve to nothing.
fn icon_exe(raw: &str) -> Option<PathBuf> {
    let mut path = raw.trim();
    if let Some((head, tail)) = path.rsplit_once(',') {
        if tail
            .trim()
            .trim_start_matches('-')
            .chars()
            .all(|c| c.is_ascii_digit())
        {
            path = head;
        }
    }
    let path = path.trim().trim_matches('"');
    (path.to_ascii_lowercase().ends_with(".exe") && !path.contains('%'))
        .then(|| PathBuf::from(path))
}

/// `20240131` -> `2024-01-31`; anything else is dropped rather than guessed.
fn install_date(raw: &str) -> Option<String> {
    let digits = raw.trim();
    if digits.len() != 8 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (year, month, day) = (&digits[..4], &digits[4..6], &digits[6..]);
    let valid = (1990..=2100).contains(&year.parse::<u32>().ok()?)
        && (1..=12).contains(&month.parse::<u32>().ok()?)
        && (1..=31).contains(&day.parse::<u32>().ok()?);
    valid.then(|| format!("{year}-{month}-{day}"))
}

/// Build one Win32 row, or `None` for entries the page must not list.
fn parse_win32(tag: &str, key: &str, values: &[RegValue]) -> Option<Row> {
    if dword(values, "SystemComponent") == Some(1) || text(values, "ParentKeyName").is_some() {
        return None;
    }
    let name = text(values, "DisplayName")?;
    let exe = text(values, "DisplayIcon").and_then(|raw| icon_exe(&raw));
    let install_folder = text(values, "InstallLocation")
        .and_then(|raw| clean_folder(&raw))
        .or_else(|| {
            exe.as_deref()
                .and_then(Path::parent)
                .and_then(|dir| clean_folder(&dir.to_string_lossy()))
        });
    let date = text(values, "InstallDate")
        .or_else(|| dword(values, "InstallDate").map(|n| n.to_string()))
        .and_then(|raw| install_date(&raw));
    let entry = AppEntry {
        id: format!("win32:{tag}:{key}"),
        source: AppSource::Win32,
        name,
        publisher: text(values, "Publisher"),
        version: text(values, "DisplayVersion"),
        install_date: date,
        size_kb: dword(values, "EstimatedSize").map(u64::from),
        install_folder,
        family: None,
        kind: PackageKind::App,
        can_check_signer: exe.is_some(),
    };
    Some((entry, exe))
}

/// Shared runtimes other apps depend on, recognised by package name.
fn is_framework(package: &str) -> bool {
    let lower = package.to_ascii_lowercase();
    [
        "framework",
        "vclibs",
        "net.native",
        "ui.xaml",
        "windowsappruntime",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Build one Store row from its package full name (`Name_Ver_Arch_Res_Pub`).
fn parse_store(full_name: &str, values: &[RegValue]) -> AppEntry {
    let parts: Vec<&str> = full_name.split('_').collect();
    let (package, version, resource, publisher_id) = match parts.as_slice() {
        [name, version, _arch, resource, publisher_id] => {
            (*name, Some(*version), *resource, Some(*publisher_id))
        }
        _ => (full_name, None, "", None),
    };
    let family = publisher_id.map_or_else(|| package.to_owned(), |id| format!("{package}_{id}"));
    // `@{...ms-resource:...}` is an unresolved resource string: show the family.
    let name = text(values, "DisplayName")
        .filter(|n| !n.starts_with("@{"))
        .unwrap_or_else(|| family.clone());
    let kind = if !resource.is_empty() && !resource.eq_ignore_ascii_case("neutral") {
        PackageKind::Resource
    } else if is_framework(package) {
        PackageKind::Framework
    } else {
        PackageKind::App
    };
    AppEntry {
        id: format!("store:{full_name}"),
        source: AppSource::Store,
        name,
        publisher: None,
        version: version.map(str::to_owned),
        install_date: None,
        size_kb: None,
        install_folder: text(values, "PackageRootFolder").and_then(|raw| clean_folder(&raw)),
        family: Some(family),
        kind,
        can_check_signer: false,
    }
}

/// Keep the first of each `name + version` (case-insensitive).
fn dedupe(rows: Vec<Row>) -> Vec<Row> {
    let mut seen = HashSet::new();
    rows.into_iter()
        .filter(|(entry, _)| {
            seen.insert((
                entry.name.to_lowercase(),
                entry.version.clone().unwrap_or_default().to_lowercase(),
            ))
        })
        .collect()
}

fn sort_by_name(rows: &mut [Row]) {
    rows.sort_by_cached_key(|(entry, _)| (entry.name.to_lowercase(), entry.id.clone()));
}

/// Read both sources from `registry`. A location that cannot be read becomes a
/// note, not a failure: a partial list is still useful.
fn collect(registry: &dyn Registry) -> (Vec<Row>, Vec<String>) {
    let mut notes = Vec::new();
    let mut win32 = Vec::new();
    for (hive, root, tag) in [
        (Hive::LocalMachine, UNINSTALL, "HKLM"),
        (Hive::LocalMachine, UNINSTALL_WOW, "HKLM32"),
        (Hive::CurrentUser, UNINSTALL, "HKCU"),
    ] {
        let keys = match registry.subkeys(hive, root) {
            Ok(keys) => keys,
            Err(err) => {
                notes.push(format!("{tag}: {err}"));
                continue;
            }
        };
        for key in keys {
            // One unreadable entry must not hide the others.
            if let Ok(values) = registry.values(hive, &format!(r"{root}\{key}")) {
                win32.extend(parse_win32(tag, &key, &values));
            }
        }
    }
    let mut win32 = dedupe(win32);
    sort_by_name(&mut win32);

    let mut store = Vec::new();
    match registry.subkeys(Hive::CurrentUser, APPX_PACKAGES) {
        Ok(keys) => {
            for key in keys {
                let path = format!(r"{APPX_PACKAGES}\{key}");
                if let Ok(values) = registry.values(Hive::CurrentUser, &path) {
                    store.push((parse_store(&key, &values), None));
                }
            }
        }
        Err(err) => notes.push(format!("Store packages: {err}")),
    }
    let mut store = dedupe(store);
    sort_by_name(&mut store);

    win32.extend(store);
    (win32, notes)
}

/// The module's runtime service: reads the registry, remembers which exe each
/// row's signer lookup should use, and opens Settings.
pub struct AppsService {
    registry: Arc<dyn Registry>,
    signatures: Arc<dyn SignatureProbe>,
    launcher: Arc<dyn PanelLauncher>,
    exes: Mutex<HashMap<String, PathBuf>>,
}

impl AppsService {
    /// Build the service over the given platform seams.
    pub fn new(
        registry: Arc<dyn Registry>,
        signatures: Arc<dyn SignatureProbe>,
        launcher: Arc<dyn PanelLauncher>,
    ) -> Self {
        Self {
            registry,
            signatures,
            launcher,
            exes: Mutex::new(HashMap::new()),
        }
    }

    /// List installed apps. Read-only.
    pub fn list(&self) -> AppsList {
        let (rows, notes) = collect(self.registry.as_ref());
        let mut exes = self.exes.lock().unwrap_or_else(PoisonError::into_inner);
        exes.clear();
        let entries = rows
            .into_iter()
            .map(|(entry, exe)| {
                if let Some(exe) = exe {
                    exes.insert(entry.id.clone(), exe);
                }
                entry
            })
            .collect();
        AppsList { entries, notes }
    }

    /// Authenticode signer of one row's exe.
    ///
    /// # Errors
    /// [`CoreError::UnknownApp`] when the id has no resolvable exe in the last list.
    pub fn signer(&self, id: &str) -> Result<SignerInfo> {
        let exe = self
            .exes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::UnknownApp(id.to_owned()))?;
        Ok(match self.signatures.check(&exe) {
            Signature::Signed { publisher } => SignerInfo {
                state: SignerState::Signed,
                publisher,
            },
            Signature::Unsigned => SignerInfo {
                state: SignerState::Unsigned,
                publisher: None,
            },
            Signature::Unknown { .. } => SignerInfo {
                state: SignerState::Unknown,
                publisher: None,
            },
        })
    }

    /// Open Windows' own Apps settings page.
    ///
    /// # Errors
    /// Returns an error when the shell could not open it or this build may not.
    pub fn open_settings(&self) -> Result<()> {
        self.launcher.open(Path::new(SETTINGS_URI), None)
    }
}

/// The module manifest.
#[derive(Debug, Default)]
pub struct AppsModule;

impl ToolModule for AppsModule {
    fn id(&self) -> &'static str {
        "apps"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Installed Apps".to_owned(),
            description: "See what is installed and who ships it. Read-only.".to_owned(),
            icon: "apps".to_owned(),
            category: ModuleCategory::Apps,
            capabilities: ModuleCapabilities {
                has_scan: false,
                has_execute: false,
                has_settings: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::{
        FakePanelLauncher, MapRegistry, MapSignatures, REG_TYPE_EXPAND_SZ, REG_TYPE_SZ,
    };

    fn sz(name: &str, value: &str) -> RegValue {
        RegValue::string(name, REG_TYPE_SZ, value)
    }

    fn registry(keys: Vec<(Hive, String, Vec<RegValue>)>) -> MapRegistry {
        keys.into_iter().fold(MapRegistry::new(), |reg, (h, p, v)| reg.with_key(h, &p, v))
    }

    fn win32_key(hive: Hive, root: &str, key: &str, values: Vec<RegValue>) -> (Hive, String, Vec<RegValue>) {
        (hive, format!(r"{root}\{key}"), values)
    }

    fn names(list: &AppsList) -> Vec<&str> {
        list.entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn service(reg: MapRegistry, sigs: MapSignatures) -> (AppsService, Arc<FakePanelLauncher>) {
        let launcher = Arc::new(FakePanelLauncher::default());
        let svc = AppsService::new(
            Arc::new(reg),
            Arc::new(sigs),
            Arc::clone(&launcher) as Arc<dyn PanelLauncher>,
        );
        (svc, launcher)
    }

    #[test]
    fn parses_fields_and_skips_hidden_entries() {
        let reg = registry(vec![
            win32_key(
                Hive::LocalMachine,
                UNINSTALL,
                "zip",
                vec![
                    sz("DisplayName", "7-Zip"),
                    sz("Publisher", "Igor Pavlov"),
                    sz("DisplayVersion", "19.00"),
                    sz("InstallDate", "20240131"),
                    RegValue::dword("EstimatedSize", 5082),
                    sz("InstallLocation", r"C:\Program Files\7-Zip\"),
                    sz("DisplayIcon", r#""C:\Program Files\7-Zip\7zFM.exe",0"#),
                ],
            ),
            win32_key(
                Hive::LocalMachine,
                UNINSTALL,
                "sys",
                vec![sz("DisplayName", "Hidden"), RegValue::dword("SystemComponent", 1)],
            ),
            win32_key(Hive::LocalMachine, UNINSTALL, "noname", vec![sz("Publisher", "X")]),
            win32_key(
                Hive::LocalMachine,
                UNINSTALL,
                "patch",
                vec![sz("DisplayName", "Update for X"), sz("ParentKeyName", "X")],
            ),
        ]);
        let (svc, _) = service(reg, MapSignatures::new());
        let list = svc.list();
        assert_eq!(names(&list), ["7-Zip"]);
        let zip = &list.entries[0];
        assert_eq!(zip.publisher.as_deref(), Some("Igor Pavlov"));
        assert_eq!(zip.install_date.as_deref(), Some("2024-01-31"));
        assert_eq!(zip.size_kb, Some(5082));
        assert_eq!(zip.install_folder.as_deref(), Some(r"C:\Program Files\7-Zip"));
        assert!(zip.can_check_signer);
    }

    #[test]
    fn dedupes_by_name_and_version_across_roots() {
        let entry = |v: &str| vec![sz("DisplayName", "Tool"), sz("DisplayVersion", v)];
        let reg = registry(vec![
            win32_key(Hive::LocalMachine, UNINSTALL, "a", entry("1.0")),
            win32_key(Hive::LocalMachine, UNINSTALL_WOW, "a", entry("1.0")),
            win32_key(Hive::CurrentUser, UNINSTALL, "a", entry("2.0")),
        ]);
        let (svc, _) = service(reg, MapSignatures::new());
        let list = svc.list();
        assert_eq!(list.entries.len(), 2);
        let v1 = list.entries.iter().find(|e| e.version.as_deref() == Some("1.0"));
        assert!(v1.expect("1.0 kept").id.starts_with("win32:HKLM:"));
    }

    #[test]
    fn folder_falls_back_to_exe_dir_and_rejects_unexpanded_paths() {
        assert_eq!(icon_exe(r"C:\a\b.ico"), None);
        assert_eq!(icon_exe(r"%SystemRoot%\x.exe"), None);
        assert_eq!(icon_exe(r"C:\a\b.exe,-3"), Some(PathBuf::from(r"C:\a\b.exe")));
        let reg = registry(vec![win32_key(
            Hive::CurrentUser,
            UNINSTALL,
            "k",
            vec![
                sz("DisplayName", "NoLoc"),
                sz("DisplayIcon", r"C:\Apps\NoLoc\noloc.exe"),
                RegValue::string("InstallLocation", REG_TYPE_EXPAND_SZ, r"%ProgramFiles%\NoLoc"),
            ],
        )]);
        let (svc, _) = service(reg, MapSignatures::new());
        assert_eq!(svc.list().entries[0].install_folder.as_deref(), Some(r"C:\Apps\NoLoc"));
    }

    #[test]
    fn bad_install_dates_are_dropped() {
        for bad in ["", "2024", "20241345", "abcdefgh", "18000101"] {
            assert_eq!(install_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn store_packages_are_classified_and_unresolved_names_use_the_family() {
        let pkg = |key: &str, display: &str| {
            (
                Hive::CurrentUser,
                format!(r"{APPX_PACKAGES}\{key}"),
                vec![
                    sz("DisplayName", display),
                    sz("PackageRootFolder", r"C:\Program Files\WindowsApps\x"),
                ],
            )
        };
        let reg = registry(vec![
            pkg("91750d7e.slack_4.52.0.0_x64__8she8kybcnzg4", "Slack"),
            pkg("microsoft.net.native.runtime.2.2_2.2.1.0_x64__8wekyb3d8bbwe", "Runtime"),
            pkg("foo.app_1.0.0.0_neutral_split.scale-100_abc123", "Foo scale"),
            pkg("sys.thing_10.0.0.0_neutral_neutral_cw5n1h2txyewy", "@{Sys.Thing_x?ms-resource://a/b}"),
        ]);
        let (svc, _) = service(reg, MapSignatures::new());
        let list = svc.list();
        let by = |n: &str| list.entries.iter().find(|e| e.name == n).expect(n);
        assert_eq!(by("Slack").kind, PackageKind::App);
        assert_eq!(by("Runtime").kind, PackageKind::Framework);
        assert_eq!(by("Foo scale").kind, PackageKind::Resource);
        let sys = by("sys.thing_cw5n1h2txyewy");
        assert_eq!(sys.kind, PackageKind::App);
        assert_eq!(sys.version.as_deref(), Some("10.0.0.0"));
        assert!(list.entries.iter().all(|e| e.source == AppSource::Store));
    }

    #[test]
    fn signer_lookup_is_lazy_and_only_for_known_rows() {
        let exe = r"C:\Apps\a\a.exe";
        let reg = registry(vec![
            win32_key(
                Hive::LocalMachine,
                UNINSTALL,
                "a",
                vec![sz("DisplayName", "A"), sz("DisplayIcon", exe)],
            ),
            win32_key(Hive::LocalMachine, UNINSTALL, "b", vec![sz("DisplayName", "B")]),
        ]);
        let sigs = MapSignatures::new().with(
            exe,
            Signature::Signed {
                publisher: Some("Acme".into()),
            },
        );
        let (svc, _) = service(reg, sigs);
        assert!(matches!(svc.signer("win32:HKLM:a"), Err(CoreError::UnknownApp(_))), "before list");
        let list = svc.list();
        assert!(!list.entries[1].can_check_signer);
        let ok = svc.signer(&list.entries[0].id).expect("signer");
        assert_eq!((ok.state, ok.publisher.as_deref()), (SignerState::Signed, Some("Acme")));
        assert!(svc.signer(&list.entries[1].id).is_err());
    }

    #[test]
    fn unsigned_exe_is_reported_unsigned() {
        let reg = registry(vec![win32_key(
            Hive::CurrentUser,
            UNINSTALL,
            "u",
            vec![sz("DisplayName", "U"), sz("DisplayIcon", r"C:\Apps\u.exe")],
        )]);
        let (svc, _) = service(reg, MapSignatures::new());
        let id = svc.list().entries[0].id.clone();
        assert_eq!(svc.signer(&id).expect("signer").state, SignerState::Unsigned);
    }

    #[test]
    fn open_settings_launches_the_apps_page_only() {
        let (svc, launcher) = service(MapRegistry::new(), MapSignatures::new());
        svc.open_settings().expect("open");
        let launches = launcher.launches();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].file, PathBuf::from("ms-settings:appsfeatures"));
        assert_eq!(launches[0].argument, None);
    }
}
