//! Uninstaller+: find app-data folders that no installed program claims
//! (research C5).
//!
//! # What this is not
//!
//! It does **not** run any uninstaller and does **not** write the registry.
//! It is not a registry cleaner: leftovers are files and folders only. The
//! Uninstall keys under HKLM/HKCU are only *read*, to learn which apps are
//! still installed.
//!
//! # What a leftover is
//!
//! An immediate child folder of `%AppData%`, `%LocalAppData%`, `%ProgramData%`
//! or `Program Files*` whose name matches no installed app. Matching is
//! deliberately generous towards "installed" (see [`is_installed`]): a folder
//! that might belong to an installed app is never reported, so a missed
//! leftover is the failure mode, not a broken app.
//!
//! # Where the opinion lives
//!
//! The scan is facts only. `analyze` applies the hard gates (protected paths,
//! anything above one, `Program Files*`, links inside) and then the
//! `uninstaller-leftovers` pack, whose only actionable rule tops out at Needs
//! Review. Nothing here is ever Safe.
//!
//! # Executing
//!
//! A leftover is moved to quarantine as one restorable folder through the
//! shared executor, which calls [`recheck_refusal`] right before the move
//! (decision K-030). Anything it cannot establish refuses the move.

use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::brain::{Brain, ProtectedPaths, Signals};
use crate::core::error::Result;
use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::path_compare::{same_path, CanonicalPath};
use crate::core::platform::{EnvResolver, Hive, Registry};
use crate::core::scanner::CancellationToken;
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket,
};

/// Module id, also the pack's `module` value and the UI route.
pub const MODULE_ID: &str = "uninstaller";

/// Id of the rule pack that rates findings.
pub const PACK_ID: &str = "uninstaller-leftovers";

/// Confidence reported for a gate verdict that is not a score.
const GATE_CONFIDENCE: u8 = 100;

/// Normalised folder names shorter than this match too much to judge.
const MIN_NAME_LEN: usize = 3;

/// Words from an installed app's names shorter than this are too common to
/// claim a folder on their own ("app", "inc", "the").
const MIN_WORD_LEN: usize = 4;

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_WOW: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_ROOTS: [(Hive, &str); 3] = [
    (Hive::LocalMachine, UNINSTALL),
    (Hive::LocalMachine, UNINSTALL_WOW),
    (Hive::CurrentUser, UNINSTALL),
];

/// Folders Windows itself keeps in these roots; never reported.
const SKIP_NAMES: [&str; 17] = [
    "packages",
    "temp",
    "programs",
    "connecteddevicesplatform",
    "crashdumps",
    "d3dscache",
    "comms",
    "publishers",
    "placeholdertilelogofolder",
    "virtualstore",
    "application data",
    "history",
    "temporary internet files",
    "package cache",
    "usoshared",
    "usoprivate",
    "softwaredistribution",
];
const SKIP_PREFIXES: [&str; 3] = ["microsoft", "windows", "regid."];

const ATTR_REPARSE_POINT: u32 = 0x0000_0400;
const ATTR_RECALL_ON_OPEN: u32 = 0x0004_0000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// Extra facts attached to every finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverMeta {
    /// Files inside, counted all the way down.
    pub files: u64,
    /// A link or cloud placeholder lives somewhere inside.
    pub has_reparse: bool,
    /// Part of the tree could not be read.
    pub incomplete: bool,
}

/// What a scan produced.
#[derive(Debug, Clone, Default)]
pub struct ScanOutput {
    /// Folders no installed app claims.
    pub findings: Vec<Finding>,
    /// Roots that could not be listed.
    pub unreadable: Vec<PathBuf>,
    /// Installed apps read from the Uninstall keys.
    pub installed_apps: u64,
    /// Whether the scan was cancelled part-way.
    pub cancelled: bool,
}

/// Progress callback: folders looked at so far and the current one.
pub type ProgressSink<'a> = &'a mut dyn FnMut(u64, &Path);

/// Everything a scan needs.
pub struct ScanContext<'a> {
    /// Folders whose immediate children are candidates.
    pub roots: &'a [PathBuf],
    /// The Uninstall keys are read from here; never written.
    pub registry: &'a dyn Registry,
    /// Cancellation.
    pub cancel: &'a CancellationToken,
}

/// Names the installed apps go by, normalised.
#[derive(Debug, Default)]
struct InstalledMarks {
    full: Vec<String>,
    words: Vec<String>,
    apps: u64,
}

/// Lowercase letters and digits only: "Foo Bar-2" and "foobar2" are one name.
fn normalise(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Last component of a path held in a registry value, quotes and an icon
/// index (`,0`) stripped.
fn leaf(text: &str) -> Option<String> {
    let trimmed = text.trim().trim_matches('"');
    let path = trimmed.split(',').next().unwrap_or(trimmed).trim_matches('"');
    let path = Path::new(path.trim_end_matches(['\\', '/']));
    path.file_name().map(|name| name.to_string_lossy().into_owned())
}

impl InstalledMarks {
    fn add(&mut self, raw: &str) {
        let full = normalise(raw);
        if full.is_empty() {
            return;
        }
        self.full.push(full);
        self.words.extend(
            raw.split(|c: char| !c.is_alphanumeric())
                .map(normalise)
                .filter(|word| word.chars().count() >= MIN_WORD_LEN),
        );
    }
}

/// Read every Uninstall entry. Any read error fails the whole thing: a
/// partial list would turn installed apps into "leftovers".
fn installed_marks(registry: &dyn Registry) -> Result<InstalledMarks> {
    let mut marks = InstalledMarks::default();
    for (hive, root) in UNINSTALL_ROOTS {
        for key in registry.subkeys(hive, root)? {
            marks.add(&key);
            let values = registry.values(hive, &format!(r"{root}\{key}"))?;
            let text = |name: &str| {
                values
                    .iter()
                    .find(|value| value.name.eq_ignore_ascii_case(name))
                    .and_then(|value| value.as_string())
            };
            if let Some(name) = text("DisplayName") {
                marks.apps += 1;
                marks.add(&name);
            }
            if let Some(publisher) = text("Publisher") {
                marks.add(&publisher);
            }
            if let Some(location) = text("InstallLocation").as_deref().and_then(leaf) {
                marks.add(&location);
            }
            if let Some(icon) = text("DisplayIcon") {
                let folder = icon
                    .trim()
                    .trim_matches('"')
                    .split(',')
                    .next()
                    .and_then(|exe| Path::new(exe.trim_matches('"')).parent())
                    .and_then(|parent| leaf(&parent.to_string_lossy()));
                if let Some(folder) = folder {
                    marks.add(&folder);
                }
            }
        }
    }
    Ok(marks)
}

/// Whether some installed app could own a folder called `name`. Generous on
/// purpose: a folder name contained in an app's name, or an app's word of
/// four or more letters contained in the folder name, both count.
fn is_installed(name: &str, marks: &InstalledMarks) -> bool {
    let folder = normalise(name);
    marks.full.iter().any(|mark| mark.contains(&folder))
        || marks.words.iter().any(|word| folder.contains(word.as_str()))
}

/// Folders Windows keeps, and names too short to judge.
fn skip_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    SKIP_NAMES.contains(&lower.as_str())
        || SKIP_PREFIXES.iter().any(|prefix| lower.starts_with(prefix))
        || normalise(name).chars().count() < MIN_NAME_LEN
}

/// Scan the roots for leftover folders. Read-only.
///
/// # Errors
/// When the Uninstall keys cannot be read: without them every folder would
/// look like a leftover, so the scan reports nothing rather than that.
pub fn scan(ctx: &ScanContext<'_>, mut progress: Option<ProgressSink<'_>>) -> Result<ScanOutput> {
    let marks = installed_marks(ctx.registry)?;
    let mut out = ScanOutput {
        installed_apps: marks.apps,
        ..ScanOutput::default()
    };
    let mut visited = 0u64;
    for root in ctx.roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            out.unreadable.push(root.clone());
            continue;
        };
        for entry in entries.flatten() {
            if ctx.cancel.is_cancelled() {
                out.cancelled = true;
                out.findings.clear();
                return Ok(out);
            }
            let path = entry.path();
            visited += 1;
            if let Some(sink) = progress.as_mut() {
                sink(visited, &path);
            }
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if !metadata.is_dir() || is_opaque(&metadata) || skip_name(&name) {
                continue;
            }
            if is_installed(&name, &marks) {
                continue;
            }
            out.findings.push(report(&path, &metadata));
        }
    }
    Ok(out)
}

fn report(path: &Path, metadata: &std::fs::Metadata) -> Finding {
    let mut finding = Finding::new(MODULE_ID, path, FindingKind::Directory);
    finding.created = metadata.created().ok().map(DateTime::<Utc>::from);
    finding.modified = metadata.modified().ok().map(DateTime::<Utc>::from);
    finding.accessed = metadata.accessed().ok().map(DateTime::<Utc>::from);
    let tree = measure(path);
    finding.size_bytes = tree.bytes;
    finding.meta = serde_json::to_value(LeftoverMeta {
        files: tree.files,
        has_reparse: tree.has_reparse,
        incomplete: tree.incomplete,
    })
    .unwrap_or(serde_json::Value::Null);
    finding
}

#[derive(Debug, Default)]
struct Tree {
    bytes: u64,
    files: u64,
    has_reparse: bool,
    incomplete: bool,
}

/// Walk `dir` without following any link or placeholder.
fn measure(dir: &Path) -> Tree {
    let mut tree = Tree::default();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            tree.incomplete = true;
            continue;
        };
        for entry in entries {
            let Ok(metadata) = entry.and_then(|entry| {
                let path = entry.path();
                std::fs::symlink_metadata(&path).map(|metadata| (path, metadata))
            }) else {
                tree.incomplete = true;
                continue;
            };
            let (path, metadata) = metadata;
            if is_opaque(&metadata) || metadata.is_symlink() {
                tree.has_reparse = true;
            } else if metadata.is_dir() {
                stack.push(path);
            } else {
                tree.files += 1;
                tree.bytes += metadata.len();
            }
        }
    }
    tree
}

#[cfg(windows)]
fn attributes(metadata: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
}

#[cfg(not(windows))]
fn attributes(_metadata: &std::fs::Metadata) -> u32 {
    0
}

/// A reparse point or a cloud placeholder: never followed, never moved.
fn is_opaque(metadata: &std::fs::Metadata) -> bool {
    attributes(metadata) & (ATTR_REPARSE_POINT | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS)
        != 0
}

/// `Program Files` and `Program Files (x86)` anywhere in the path: never
/// actionable here, whatever the protected list says.
fn under_program_files(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(part) => {
            let part = part.to_string_lossy();
            part.eq_ignore_ascii_case("Program Files")
                || part.eq_ignore_ascii_case("Program Files (x86)")
        }
        _ => false,
    })
}

/// A protected location at or beneath `path` makes `path` an ancestor of it.
fn protected_below(path: &Path, protected: &ProtectedPaths) -> Option<String> {
    let outer = CanonicalPath::lexical(path);
    protected
        .entries()
        .iter()
        .find(|entry| CanonicalPath::lexical(&entry.path).is_within(&outer))
        .map(|entry| format!("this folder contains a protected location: {}", entry.reason))
}

/// The folders a leftover may sit directly in: `%APPDATA%`, `%LOCALAPPDATA%`,
/// `%ProgramData%` and both Program Files folders. The scan reads only these,
/// and the executor moves only their direct children.
pub fn known_roots(env: &dyn EnvResolver) -> Vec<PathBuf> {
    [
        "APPDATA",
        "LOCALAPPDATA",
        "ProgramData",
        "ProgramFiles",
        "ProgramFiles(x86)",
    ]
    .into_iter()
    .filter_map(|name| env.var(name))
    .map(PathBuf::from)
    .collect()
}

/// Whether `path` is a direct child of one of the [`known_roots`]. A folder
/// deeper down belongs to whatever app owns its parent, installed or not.
fn is_top_level(path: &Path, env: &dyn EnvResolver) -> bool {
    path.parent()
        .is_some_and(|parent| known_roots(env).iter().any(|root| same_path(parent, root)))
}

/// The hard gates `analyze` and [`recheck_refusal`] share.
fn gate_refusal(path: &Path, brain: &Brain) -> Option<String> {
    if under_program_files(path) {
        return Some(
            "Folders under Program Files are shown for information only; remove them with the app's own uninstaller."
                .to_owned(),
        );
    }
    if let Some(refusal) = brain.path_refusal(path) {
        return Some(refusal.reason);
    }
    protected_below(path, brain.protected_paths())
}

/// Why the executor must not move `path` right now, or `None` when every
/// check still holds. Fails closed: anything it cannot establish refuses.
///
/// Called by the executor immediately before the quarantine move.
pub fn recheck_refusal(
    path: &Path,
    registry: &dyn Registry,
    env: &dyn EnvResolver,
    brain: &Brain,
) -> Option<String> {
    let shown = path.display();
    if let Some(reason) = gate_refusal(path, brain) {
        return Some(format!("skipped: {reason}"));
    }
    if !is_top_level(path, env) {
        return Some(format!(
            "skipped: `{shown}` does not sit directly in AppData, LocalAppData or ProgramData, so it was not moved"
        ));
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) => return Some(format!("skipped: `{shown}` could not be re-checked: {err}")),
    };
    if !metadata.is_dir() || is_opaque(&metadata) {
        return Some(format!("skipped: `{shown}` is no longer an ordinary folder"));
    }
    let tree = measure(path);
    if tree.has_reparse || tree.incomplete {
        return Some(format!(
            "skipped: `{shown}` holds a link, a cloud placeholder or something that cannot be read, so it was not moved"
        ));
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if skip_name(&name) {
        return Some(format!("skipped: `{shown}` is a folder Windows keeps"));
    }
    match installed_marks(registry) {
        Err(err) => Some(format!(
            "skipped: the list of installed apps could not be read ({err}), so `{shown}` was not moved"
        )),
        Ok(marks) if is_installed(&name, &marks) => Some(format!(
            "skipped: an installed app now matches `{shown}`, so it is not a leftover"
        )),
        Ok(_) => None,
    }
}

/// Turn findings into explained recommendations. Read-only.
pub fn analyze(findings: &[Finding], brain: &Brain) -> Vec<Recommendation> {
    findings
        .iter()
        .filter_map(|finding| match recommend(finding, brain) {
            Ok(rec) => Some(rec),
            Err(err) => {
                tracing::error!(
                    path = %finding.path.display(),
                    error = %err,
                    "dropped a recommendation that failed its own invariants"
                );
                None
            }
        })
        .collect()
}

fn recommend(finding: &Finding, brain: &Brain) -> Result<Recommendation> {
    let blocked = |reason: String| {
        Recommendation::new(
            vec![finding.id],
            RiskBucket::DoNotDelete,
            GATE_CONFIDENCE,
            Action::InfoOnly,
            reason,
            0,
            Justification::Informational,
        )
    };

    // Hard gates first; no rule can talk its way past them.
    if let Some(reason) = gate_refusal(&finding.path, brain) {
        return blocked(reason);
    }
    let meta: Option<LeftoverMeta> = serde_json::from_value(finding.meta.clone()).ok();
    match meta {
        None => return blocked("This folder's contents were not recorded.".to_owned()),
        Some(meta) if meta.has_reparse => {
            return blocked(
                "This folder holds a link or a cloud placeholder, so it is shown for information only."
                    .to_owned(),
            )
        }
        Some(meta) if meta.incomplete => {
            return blocked(
                "Part of this folder could not be read, so it is shown for information only."
                    .to_owned(),
            )
        }
        Some(_) => {}
    }

    let pack = brain.packs().iter().find(|pack| pack.id() == PACK_ID);
    let rule = pack.and_then(|pack| pack.rules.iter().find(|rule| rule.matches_path(&finding.path)));
    let Some(rule) = rule else {
        return blocked(
            "No installed app matches this folder, but it is outside your own app data, so it is shown for information only."
                .to_owned(),
        );
    };
    let verdict = brain.classify_finding_with(finding, Some(rule), &Signals::unknown());
    let Some((pack_id, rule_id)) = verdict.rule.clone() else {
        return blocked(verdict.reason);
    };
    // Never green: a name match is a guess, so a human always decides.
    let bucket = match verdict.bucket {
        RiskBucket::Safe => RiskBucket::NeedsReview,
        other => other,
    };
    let action = if bucket.is_actionable() {
        verdict.action
    } else {
        Action::InfoOnly
    };
    let savings = if action == Action::Quarantine {
        finding.size_bytes
    } else {
        0
    };
    Recommendation::new(
        vec![finding.id],
        bucket,
        verdict.confidence,
        action,
        verdict.reason,
        savings,
        Justification::Rule { pack_id, rule_id },
    )
}

/// Registry entry. The commands in `commands.rs` drive the scan.
#[derive(Debug, Default)]
pub struct UninstallerModule;

impl ToolModule for UninstallerModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: MODULE_ID.to_owned(),
            name: "Uninstaller leftovers".to_owned(),
            description: "Find app-data folders left behind by apps you removed, and move them to quarantine. Runs no uninstaller and never touches the registry."
                .to_owned(),
            icon: "leftovers".to_owned(),
            category: ModuleCategory::Apps,
            capabilities: ModuleCapabilities {
                has_scan: true,
                has_execute: true,
                has_settings: false,
            },
        }
    }

    fn analyze(&self, findings: &[Finding], brain: &Brain) -> Vec<Recommendation> {
        analyze(findings, brain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::brain::rules;
    use crate::core::db;
    use crate::core::error::CoreError;
    use crate::core::executor::{
        execute, ExecEnvironment, ExecOptions, ExecUnit, NoStartupControl, NoTweakControl,
    };
    use crate::core::platform::{
        AlwaysUnlocked, MapEnv, MapRegistry, RegValue, StubProcessProbe, StubRecycleBin,
        REG_TYPE_SZ,
    };
    use crate::core::quarantine::{Quarantine, RetentionPolicy};
    use crate::core::types::ExecOutcome;
    use std::fs;
    use std::sync::Arc;

    struct Profile {
        _dir: tempfile::TempDir,
        roaming: PathBuf,
        program_files: PathBuf,
        env: MapEnv,
    }

    impl Profile {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().join("profile");
            let roaming = root.join("AppData/Roaming");
            let program_files = dir.path().join("Program Files");
            for sub in [&roaming, &root.join("AppData/Local"), &program_files] {
                fs::create_dir_all(sub).expect("dirs");
            }
            let env = MapEnv::new()
                .with("APPDATA", roaming.to_string_lossy())
                .with("LOCALAPPDATA", root.join("AppData/Local").to_string_lossy());
            Self {
                _dir: dir,
                roaming,
                program_files,
                env,
            }
        }

        fn brain(&self, protected: ProtectedPaths) -> Brain {
            let packs = rules::load_dir(&repo_rules_dir(), &self.env)
                .expect("packs")
                .into_iter()
                .filter(|pack| pack.id() == PACK_ID)
                .collect();
            Brain::builder()
                .protected_paths(protected)
                .packs(packs)
                .lock_probe(Arc::new(AlwaysUnlocked))
                .process_probe(Arc::new(StubProcessProbe::none()))
                .build()
        }

        fn scan(&self, registry: &dyn Registry) -> ScanOutput {
            let roots = [self.roaming.clone(), self.program_files.clone()];
            let cancel = CancellationToken::new();
            scan(
                &ScanContext {
                    roots: &roots,
                    registry,
                    cancel: &cancel,
                },
                None,
            )
            .expect("scan")
        }
    }

    fn repo_rules_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root")
            .join("rules")
    }

    fn app(registry: MapRegistry, key: &str, name: &str) -> MapRegistry {
        registry.with_key(
            Hive::LocalMachine,
            &format!(r"{UNINSTALL}\{key}"),
            vec![RegValue::string("DisplayName", REG_TYPE_SZ, name)],
        )
    }

    fn installed() -> MapRegistry {
        app(MapRegistry::new(), "{0000-1111}", "Mozilla Firefox (x64 en-US)")
    }

    fn marks(registry: &MapRegistry) -> InstalledMarks {
        installed_marks(registry).expect("marks")
    }

    /// Every read fails.
    struct BrokenRegistry;

    impl Registry for BrokenRegistry {
        fn values(&self, _: Hive, _: &str) -> Result<Vec<RegValue>> {
            Err(CoreError::Quarantine("registry unreadable".to_owned()))
        }
        fn value(&self, _: Hive, _: &str, _: &str) -> Result<Option<RegValue>> {
            Err(CoreError::Quarantine("registry unreadable".to_owned()))
        }
        fn set_value(&self, _: Hive, _: &str, _: &RegValue) -> Result<()> {
            Err(CoreError::Quarantine("no writes".to_owned()))
        }
        fn delete_value(&self, _: Hive, _: &str, _: &str) -> Result<()> {
            Err(CoreError::Quarantine("no writes".to_owned()))
        }
        fn subkeys(&self, _: Hive, _: &str) -> Result<Vec<String>> {
            Err(CoreError::Quarantine("registry unreadable".to_owned()))
        }
    }

    fn names(out: &ScanOutput) -> Vec<String> {
        let mut names: Vec<String> = out
            .findings
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn matching_is_generous_towards_installed() {
        let registry = installed().with_key(
            Hive::CurrentUser,
            &format!(r"{UNINSTALL}\Zoom"),
            vec![
                RegValue::string("DisplayName", REG_TYPE_SZ, "Zoom Workplace"),
                RegValue::string("Publisher", REG_TYPE_SZ, "Zoom Video Communications"),
                RegValue::string(
                    "DisplayIcon",
                    REG_TYPE_SZ,
                    r#""C:\Users\me\AppData\Roaming\ZoomBin\Zoom.exe",0"#,
                ),
            ],
        );
        let marks = marks(&registry);
        assert_eq!(marks.apps, 2);
        for name in ["Mozilla", "Firefox", "Mozilla Firefox", "zoom", "ZoomBin", "Zoom-Video"] {
            assert!(is_installed(name, &marks), "{name} should count as installed");
        }
        for name in ["OldEditor", "GreenShot2", "Notepad3"] {
            assert!(!is_installed(name, &marks), "{name} should not match");
        }
    }

    #[test]
    fn windows_folders_and_short_names_are_skipped() {
        for name in ["Microsoft", "Windows Defender", "Packages", "Temp", "ab", "regid.1991-06.com.microsoft"] {
            assert!(skip_name(name), "{name}");
        }
        assert!(!skip_name("OldEditor"));
    }

    #[test]
    fn only_unclaimed_folders_are_reported() {
        let p = Profile::new();
        for name in ["Mozilla", "OldEditor", "Microsoft"] {
            fs::create_dir_all(p.roaming.join(name)).expect("dir");
        }
        fs::write(p.roaming.join("OldEditor/settings.json"), b"{}").expect("file");
        fs::write(p.roaming.join("loose.txt"), b"x").expect("file");

        let out = p.scan(&installed());

        assert_eq!(names(&out), vec!["OldEditor"]);
        assert_eq!(out.findings[0].size_bytes, 2);
        assert_eq!(out.installed_apps, 1);
    }

    #[test]
    fn an_unreadable_registry_reports_nothing() {
        let roots = [PathBuf::from(".")];
        let cancel = CancellationToken::new();
        let result = scan(
            &ScanContext {
                roots: &roots,
                registry: &BrokenRegistry,
                cancel: &cancel,
            },
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn app_data_leftovers_are_review_at_most_and_quarantined() {
        let p = Profile::new();
        fs::create_dir_all(p.roaming.join("OldEditor")).expect("dir");
        let out = p.scan(&installed());

        let recs = analyze(&out.findings, &p.brain(ProtectedPaths::empty()));

        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].risk, RiskBucket::NeedsReview);
        assert_eq!(recs[0].action, Action::Quarantine);
        assert!(!recs[0].reason.is_empty());
        assert_ne!(recs[0].risk, RiskBucket::Safe);
    }

    #[test]
    fn program_files_and_protected_paths_are_info_only() {
        let p = Profile::new();
        fs::create_dir_all(p.program_files.join("OldTool")).expect("dir");
        fs::create_dir_all(p.roaming.join("OldEditor")).expect("dir");
        let out = p.scan(&installed());
        let brain = p.brain(ProtectedPaths::from_trees([p.roaming.join("OldEditor")]));

        let recs = analyze(&out.findings, &brain);

        assert_eq!(recs.len(), 2);
        for rec in &recs {
            assert_eq!(rec.risk, RiskBucket::DoNotDelete, "{rec:?}");
            assert_eq!(rec.action, Action::InfoOnly, "{rec:?}");
            assert!(!rec.reason.is_empty());
        }
    }

    #[test]
    fn the_scan_changes_nothing() {
        let p = Profile::new();
        fs::create_dir_all(p.roaming.join("OldEditor/sub")).expect("dir");
        fs::write(p.roaming.join("OldEditor/sub/a.txt"), b"abc").expect("file");
        let registry = installed();

        let _ = p.scan(&registry);

        assert!(p.roaming.join("OldEditor/sub/a.txt").is_file());
        assert!(registry.writes().is_empty(), "the registry is never written");
    }

    struct Rig {
        _qdir: tempfile::TempDir,
        quarantine: Quarantine,
        conn: rusqlite::Connection,
    }

    fn rig() -> Rig {
        let qdir = tempfile::tempdir().expect("qdir");
        Rig {
            quarantine: Quarantine::with_root(qdir.path().join("q"), RetentionPolicy::default()),
            conn: db::open_in_memory().expect("db"),
            _qdir: qdir,
        }
    }

    fn run_one(
        rig: &Rig,
        brain: &Brain,
        registry: &dyn Registry,
        rec: &Recommendation,
        finding: &Finding,
        env: &dyn EnvResolver,
    ) -> crate::core::types::ExecReport {
        let bin = StubRecycleBin::default();
        let env = ExecEnvironment {
            quarantine: &rig.quarantine,
            db: &rig.conn,
            brain,
            recycle_bin: &bin,
            startup: &NoStartupControl,
            tweaks: &NoTweakControl,
            registry,
            env,
        };
        let units = [ExecUnit {
            recommendation: rec,
            finding,
        }];
        execute(&units, &env, &ExecOptions::default(), None).expect("execute")
    }

    /// A scanned `OldEditor` leftover with one file inside.
    fn leftover(p: &Profile) -> (Brain, Recommendation, Finding) {
        fs::create_dir_all(p.roaming.join("OldEditor/sub")).expect("dir");
        fs::write(p.roaming.join("OldEditor/sub/a.txt"), b"abc").expect("file");
        let out = p.scan(&installed());
        let brain = p.brain(ProtectedPaths::empty());
        let recs = analyze(&out.findings, &brain);
        assert_eq!(recs[0].action, Action::Quarantine, "{recs:?}");
        (brain, recs[0].clone(), out.findings[0].clone())
    }

    fn assert_skipped(report: &crate::core::types::ExecReport, p: &Profile) {
        assert!(
            matches!(report.items[0].outcome, ExecOutcome::Skipped { .. }),
            "{report:?}"
        );
        assert!(p.roaming.join("OldEditor/sub/a.txt").is_file(), "nothing was moved");
    }

    #[test]
    fn quarantine_then_restore_brings_back_the_whole_tree() {
        let p = Profile::new();
        let (brain, rec, finding) = leftover(&p);
        let rig = rig();

        let report = run_one(&rig, &brain, &installed(), &rec, &finding, &p.env);

        assert_eq!(report.items[0].outcome, ExecOutcome::Succeeded, "{report:?}");
        assert!(!p.roaming.join("OldEditor").exists());
        let (restored, failed) = rig
            .quarantine
            .restore_batch(&rig.conn, report.batch_id.expect("batch"))
            .expect("restore");
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(restored.len(), 1);
        assert_eq!(fs::read(p.roaming.join("OldEditor/sub/a.txt")).expect("back"), b"abc");
    }

    #[test]
    fn recheck_refuses_when_an_app_now_claims_the_folder() {
        let p = Profile::new();
        let (brain, rec, finding) = leftover(&p);
        let now = app(installed(), "OldEditor", "Old Editor 2");

        assert_skipped(&run_one(&rig(), &brain, &now, &rec, &finding, &p.env), &p);
    }

    #[test]
    fn recheck_refuses_when_the_registry_cannot_be_read() {
        let p = Profile::new();
        let (brain, rec, finding) = leftover(&p);

        assert_skipped(&run_one(&rig(), &brain, &BrokenRegistry, &rec, &finding, &p.env), &p);
    }

    #[test]
    fn recheck_refuses_a_protected_folder_or_an_ancestor_of_one() {
        let p = Profile::new();
        let (_, rec, finding) = leftover(&p);
        for tree in [p.roaming.join("OldEditor"), p.roaming.join("OldEditor/sub")] {
            let brain = p.brain(ProtectedPaths::from_trees([tree]));
            assert_skipped(&run_one(&rig(), &brain, &installed(), &rec, &finding, &p.env), &p);
        }
    }

    #[test]
    fn recheck_refuses_anything_under_program_files() {
        let p = Profile::new();
        let brain = p.brain(ProtectedPaths::empty());
        fs::create_dir_all(p.program_files.join("OldTool")).expect("dir");
        assert!(recheck_refusal(&p.program_files.join("OldTool"), &installed(), &p.env, &brain).is_some());
    }

    #[test]
    fn recheck_refuses_a_folder_below_the_top_level() {
        let p = Profile::new();
        let (brain, rec, mut finding) = leftover(&p);
        finding.path = p.roaming.join("OldEditor/sub");

        assert_skipped(&run_one(&rig(), &brain, &installed(), &rec, &finding, &p.env), &p);
        assert!(
            recheck_refusal(&p.roaming.join("OldEditor"), &installed(), &MapEnv::new(), &brain)
                .is_some(),
            "a folder outside every known root is refused"
        );
    }

    #[test]
    fn recheck_runs_whatever_the_justification() {
        let p = Profile::new();
        let (brain, mut rec, finding) = leftover(&p);
        rec.justification = Justification::DuplicateOf {
            content_hash: "abc".to_owned(),
            keeper: false,
            keeper_path: None,
        };
        let now = app(installed(), "OldEditor", "Old Editor 2");

        let report = run_one(&rig(), &brain, &now, &rec, &finding, &p.env);

        assert_skipped(&report, &p);
        let ExecOutcome::Skipped { reason } = &report.items[0].outcome else {
            unreachable!()
        };
        assert!(reason.contains("an installed app now matches"), "{reason}");
    }

    #[cfg(windows)]
    #[test]
    fn recheck_refuses_a_folder_with_a_junction_inside() {
        let p = Profile::new();
        let (brain, rec, finding) = leftover(&p);
        let target = p.roaming.join("elsewhere");
        fs::create_dir_all(&target).expect("target");
        assert!(
            crate::core::test_support::make_junction(&p.roaming.join("OldEditor/door"), &target),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );

        assert_skipped(&run_one(&rig(), &brain, &installed(), &rec, &finding, &p.env), &p);
    }

    #[test]
    fn a_folder_finding_from_another_module_is_still_refused() {
        let p = Profile::new();
        let (brain, rec, mut finding) = leftover(&p);
        finding.module = "cleaner".to_owned();

        assert_skipped(&run_one(&rig(), &brain, &installed(), &rec, &finding, &p.env), &p);
    }
}
