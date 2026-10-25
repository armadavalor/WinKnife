//! Empty Folders: find folders that hold no files at any depth (module spec 08).
//!
//! # What "empty" means here
//!
//! A folder is empty only when **nothing but empty folders lives beneath it**.
//! A hidden `desktop.ini` or a `Thumbs.db` is a file, so it breaks emptiness;
//! so does any entry the walk cannot or will not look inside:
//!
//! - an **access-denied** folder (it may hold anything),
//! - a **reparse point** (symlink, junction) or a **cloud placeholder**
//!   (`RECALL_ON_OPEN`, `RECALL_ON_DATA_ACCESS`): never followed, and it counts
//!   as content of its parent, so the parent is not offered for removal,
//! - a protected tree (never entered).
//!
//! Only the **topmost** empty folder of each empty subtree is reported; the
//! number of empty folders nested inside it is recorded in `meta.nested`.
//!
//! # Where the opinion lives
//!
//! The scan is facts only. Everything the user is told about risk comes from
//! `analyze`: the hard gates first (system folders, protected paths and anything
//! above one), then the `empty-folders` rule pack through the brain. Savings are
//! always 0 bytes; the UI shows a folder count instead.
//!
//! # Executing
//!
//! Removal is `Action::Quarantine` through the shared executor, which calls
//! [`recheck_refusal`] right before the move: a folder that was empty at scan
//! time but is not any more is skipped with a reason and never moved.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::brain::{Brain, ProtectedPaths, Signals};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::path_compare::CanonicalPath;
use crate::core::scanner::CancellationToken;
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket,
};

/// Module id, also the pack's `module` value and the UI route.
pub const MODULE_ID: &str = "empty-folders";

/// Id of the rule pack that rates findings.
pub const PACK_ID: &str = "empty-folders";

/// Deepest nesting inspected. A tree deeper than this is treated as content:
/// when in doubt, it is not empty.
const MAX_DEPTH: usize = 200;

/// Confidence reported for a gate verdict that is not a score.
const GATE_CONFIDENCE: u8 = 100;

const ATTR_SYSTEM: u32 = 0x0000_0004;
const ATTR_REPARSE_POINT: u32 = 0x0000_0400;
const ATTR_RECALL_ON_OPEN: u32 = 0x0004_0000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// Extra facts attached to every finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmptyFolderMeta {
    /// Empty folders nested inside this one, counted all the way down.
    pub nested: u64,
    /// Whether Windows marks the folder `FILE_ATTRIBUTE_SYSTEM`.
    pub system: bool,
}

/// What a scan produced.
#[derive(Debug, Clone, Default)]
pub struct ScanOutput {
    /// The topmost empty folders.
    pub findings: Vec<Finding>,
    /// Folders that could not be read (access denied or vanished). Each one is
    /// treated as not empty.
    pub unreadable: Vec<PathBuf>,
    /// Protected trees that were deliberately not entered.
    pub skipped_protected: u64,
    /// Folders looked at.
    pub folders_visited: u64,
    /// Whether the scan was cancelled part-way.
    pub cancelled: bool,
}

/// Progress callback: folders visited so far and the folder being looked at.
pub type ProgressSink<'a> = &'a mut dyn FnMut(u64, &Path);

/// Everything a scan needs.
pub struct ScanContext<'a> {
    /// Folder to scan.
    pub root: &'a Path,
    /// Trees that are never entered.
    pub protected: &'a ProtectedPaths,
    /// Cancellation.
    pub cancel: &'a CancellationToken,
}

enum Status {
    /// No files at any depth; `nested` empty folders live inside.
    Empty { nested: u64 },
    NotEmpty,
}

struct Walker<'a, 'p> {
    protected: &'a ProtectedPaths,
    cancel: &'a CancellationToken,
    progress: Option<ProgressSink<'p>>,
    out: ScanOutput,
}

/// Scan `ctx.root` for empty folders. Read-only.
///
/// A drive root is a legal scan root and is never itself a finding.
pub fn scan(ctx: &ScanContext<'_>, progress: Option<ProgressSink<'_>>) -> ScanOutput {
    let mut walker = Walker {
        protected: ctx.protected,
        cancel: ctx.cancel,
        progress,
        out: ScanOutput::default(),
    };
    if let Status::Empty { nested } = walker.inspect(ctx.root, 0) {
        // `parent()` is `None` only for a drive root.
        if ctx.root.parent().is_some() {
            walker.report(ctx.root, nested);
        }
    }
    walker.out.cancelled = ctx.cancel.is_cancelled();
    walker.out
}

impl Walker<'_, '_> {
    fn inspect(&mut self, dir: &Path, depth: usize) -> Status {
        if self.cancel.is_cancelled() || depth > MAX_DEPTH {
            return Status::NotEmpty;
        }
        self.out.folders_visited += 1;
        if let Some(sink) = self.progress.as_deref_mut() {
            sink(self.out.folders_visited, dir);
        }

        let Ok(listing) = std::fs::read_dir(dir) else {
            self.out.unreadable.push(dir.to_path_buf());
            return Status::NotEmpty;
        };

        let mut has_content = false;
        let mut empties: Vec<(PathBuf, u64)> = Vec::new();
        for entry in listing {
            if self.cancel.is_cancelled() {
                has_content = true;
                break;
            }
            let Some((path, metadata)) = entry.ok().and_then(|e| Some((e.path(), e.metadata().ok()?)))
            else {
                self.out.unreadable.push(dir.to_path_buf());
                has_content = true;
                continue;
            };
            if !metadata.is_dir() || is_opaque(&metadata) {
                has_content = true;
            } else if self.protected.reason_for(&path).is_some() {
                self.out.skipped_protected += 1;
                has_content = true;
            } else {
                match self.inspect(&path, depth + 1) {
                    Status::Empty { nested } => empties.push((path, nested)),
                    Status::NotEmpty => has_content = true,
                }
            }
        }

        if !has_content {
            let nested = empties.iter().map(|(_, n)| 1 + n).sum();
            return Status::Empty { nested };
        }
        // This folder is not empty, so each empty child is the top of its own
        // empty subtree.
        for (path, nested) in empties {
            self.report(&path, nested);
        }
        Status::NotEmpty
    }

    fn report(&mut self, path: &Path, nested: u64) {
        let mut finding = Finding::new(MODULE_ID, path, FindingKind::Directory);
        let mut system = false;
        // One authoritative stat for the clocks: the directory listing's copy
        // of a folder's timestamps is the lazy one.
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            finding.created = metadata.created().ok().map(DateTime::<Utc>::from);
            finding.modified = metadata.modified().ok().map(DateTime::<Utc>::from);
            finding.accessed = metadata.accessed().ok().map(DateTime::<Utc>::from);
            system = attributes(&metadata) & ATTR_SYSTEM != 0;
        }
        finding.meta = serde_json::to_value(EmptyFolderMeta { nested, system })
            .unwrap_or(serde_json::Value::Null);
        self.out.findings.push(finding);
    }
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

/// A reparse point or a cloud placeholder: never followed, never reported.
fn is_opaque(metadata: &std::fs::Metadata) -> bool {
    attributes(metadata) & (ATTR_REPARSE_POINT | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS)
        != 0
}

/// Why the executor must not move `path` right now, or `None` when it is still
/// an ordinary, empty folder.
///
/// Called by the executor immediately before the quarantine move. Anything it
/// cannot establish counts as a refusal.
pub fn recheck_refusal(path: &Path) -> Option<String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) => {
            return Some(format!(
                "skipped: `{}` could not be re-checked: {err}",
                path.display()
            ))
        }
    };
    if !metadata.is_dir() || is_opaque(&metadata) {
        return Some(format!(
            "skipped: `{}` is no longer an ordinary folder",
            path.display()
        ));
    }
    if attributes(&metadata) & ATTR_SYSTEM != 0 {
        return Some(format!(
            "skipped: `{}` is marked as a system folder",
            path.display()
        ));
    }

    let protected = ProtectedPaths::empty();
    let cancel = CancellationToken::new();
    let mut walker = Walker {
        protected: &protected,
        cancel: &cancel,
        progress: None,
        out: ScanOutput::default(),
    };
    match walker.inspect(path, 0) {
        Status::Empty { .. } => None,
        Status::NotEmpty => Some(format!(
            "skipped: `{}` is no longer empty (or part of it cannot be read), so it was not moved",
            path.display()
        )),
    }
}

/// Turn findings into explained recommendations. Read-only.
pub fn analyze(findings: &[Finding], brain: &Brain) -> Vec<Recommendation> {
    analyze_at(findings, brain, Utc::now())
}

/// [`analyze`] with the reference instant injected.
pub fn analyze_at(
    findings: &[Finding],
    brain: &Brain,
    now: DateTime<Utc>,
) -> Vec<Recommendation> {
    findings
        .iter()
        .filter_map(|finding| {
            match recommend(finding, brain, now) {
                Ok(rec) => Some(rec),
                Err(err) => {
                    tracing::error!(
                        path = %finding.path.display(),
                        error = %err,
                        "dropped a recommendation that failed its own invariants"
                    );
                    None
                }
            }
        })
        .collect()
}

fn recommend(
    finding: &Finding,
    brain: &Brain,
    now: DateTime<Utc>,
) -> crate::core::error::Result<Recommendation> {
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
    if let Some(refusal) = brain.path_refusal(&finding.path) {
        return blocked(refusal.reason);
    }
    if let Some(reason) = protected_below(&finding.path, brain.protected_paths()) {
        return blocked(reason);
    }
    if meta_flag(finding, "system") {
        return blocked(
            "Windows marks this as a system folder, so it is left alone even though it is empty."
                .to_owned(),
        );
    }

    let rule = pick_rule(brain, finding, now);
    let verdict = brain.classify_finding_with(finding, rule, &Signals::unknown());
    let action = if verdict.bucket.is_actionable() {
        verdict.action
    } else {
        Action::InfoOnly
    };
    let justification = match &verdict.rule {
        Some((pack_id, rule_id)) => Justification::Rule {
            pack_id: pack_id.clone(),
            rule_id: rule_id.clone(),
        },
        None => Justification::Informational,
    };
    Recommendation::new(
        vec![finding.id],
        verdict.bucket,
        verdict.confidence,
        action,
        verdict.reason,
        0,
        justification,
    )
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

fn meta_flag(finding: &Finding, key: &str) -> bool {
    finding
        .meta
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// First rule of the pack, in declaration order, whose paths and age limit fit.
fn pick_rule<'a>(
    brain: &'a Brain,
    finding: &Finding,
    now: DateTime<Utc>,
) -> Option<&'a crate::core::brain::rules::CompiledRule> {
    let pack = brain.packs().iter().find(|pack| pack.id() == PACK_ID)?;
    pack.rules.iter().find(|rule| {
        if !rule.matches_path(&finding.path) {
            return false;
        }
        match rule.spec.match_.min_age_hours {
            None => true,
            // A folder whose clock cannot be read counts as new.
            Some(min_age) => finding
                .modified
                .is_some_and(|modified| (now - modified).num_hours() >= i64::from(min_age)),
        }
    })
}

/// Registry entry. The commands in `commands.rs` drive the scan.
#[derive(Debug, Default)]
pub struct EmptyFoldersModule;

impl ToolModule for EmptyFoldersModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: MODULE_ID.to_owned(),
            name: "Empty Folders".to_owned(),
            description:
                "Find folders that contain no files at any depth, and move them to quarantine."
                    .to_owned(),
            icon: "folder".to_owned(),
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
    use crate::core::executor::{
        execute, ExecEnvironment, ExecOptions, ExecUnit, NoStartupControl,
        NoTweakControl,
    };
    use crate::core::platform::{AlwaysUnlocked, MapEnv, MapRegistry, StubProcessProbe, StubRecycleBin};
    use crate::core::quarantine::{Quarantine, RetentionPolicy};
    use crate::core::types::ExecOutcome;
    use std::fs;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    /// A miniature profile in a tempdir; every variable the shipped pack
    /// expands points into it.
    struct Profile {
        _dir: tempfile::TempDir,
        root: PathBuf,
        env: MapEnv,
    }

    impl Profile {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().join("profile");
            let temp = root.join("AppData/Local/Temp");
            for sub in ["AppData/Roaming", "ProgramData", "Documents", "Desktop"] {
                fs::create_dir_all(root.join(sub)).expect("dirs");
            }
            fs::create_dir_all(&temp).expect("temp");
            let env = MapEnv::new()
                .with("USERPROFILE", root.to_string_lossy())
                .with("TEMP", temp.to_string_lossy())
                .with("LOCALAPPDATA", root.join("AppData/Local").to_string_lossy())
                .with("APPDATA", root.join("AppData/Roaming").to_string_lossy())
                .with("PROGRAMDATA", root.join("ProgramData").to_string_lossy());
            Self {
                _dir: dir,
                root,
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

        fn scan(&self, root: &Path) -> ScanOutput {
            let protected = ProtectedPaths::empty();
            let cancel = CancellationToken::new();
            scan(
                &ScanContext {
                    root,
                    protected: &protected,
                    cancel: &cancel,
                },
                None,
            )
        }
    }

    fn repo_rules_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root")
            .join("rules")
    }

    fn mkdir(path: &Path) {
        fs::create_dir_all(path).expect("mkdir");
    }

    fn backdate_dir(path: &Path, hours: u64) {
        let when = SystemTime::now() - Duration::from_secs(hours * 3600);
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_FLAG_BACKUP_SEMANTICS lets a directory be opened for writing
            // its attributes.
            // Test-only, and the flag is not DELETE_ON_CLOSE, so the disallow
            // on `custom_flags` does not apply to what this does.
            #[allow(clippy::disallowed_methods)]
            let opened = fs::File::options()
                .write(true)
                .custom_flags(0x0200_0000)
                .open(path);
            opened
        };
        #[cfg(not(windows))]
        let file = fs::File::open(path);
        file.expect("open dir")
            .set_times(fs::FileTimes::new().set_modified(when))
            .expect("backdate dir");
    }

    fn found(out: &ScanOutput, root: &Path) -> Vec<String> {
        let mut names: Vec<_> = out
            .findings
            .iter()
            .map(|f| {
                f.path
                    .strip_prefix(root)
                    .unwrap_or(&f.path)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_nested_empty_tree_reports_only_its_top() {
        let p = Profile::new();
        let base = p.root.join("work");
        mkdir(&base.join("a/b/c"));
        mkdir(&base.join("a/d"));
        fs::write(base.join("keep.txt"), b"x").expect("file");

        let out = p.scan(&base);

        assert_eq!(found(&out, &base), vec!["a"]);
        let meta: EmptyFolderMeta =
            serde_json::from_value(out.findings[0].meta.clone()).expect("meta");
        assert_eq!(meta.nested, 3, "b, c and d are inside a");
    }

    #[test]
    fn one_hidden_file_breaks_emptiness() {
        let p = Profile::new();
        let base = p.root.join("work");
        mkdir(&base.join("a/b/c"));
        fs::write(base.join("a/b/c/desktop.ini"), b"[.ShellClassInfo]").expect("file");
        mkdir(&base.join("a/empty"));
        fs::write(base.join("keep.txt"), b"x").expect("file");

        let out = p.scan(&base);

        assert_eq!(found(&out, &base), vec!["a/empty"]);
    }

    #[test]
    fn an_empty_scan_root_is_reported_but_a_drive_root_has_no_parent() {
        let p = Profile::new();
        let base = p.root.join("lonely");
        mkdir(&base);
        assert_eq!(p.scan(&base).findings.len(), 1);
        // The guard that keeps a drive root out of the findings.
        assert!(Path::new("C:\\").parent().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_is_not_followed_and_not_reported() {
        let p = Profile::new();
        let base = p.root.join("work");
        let target = p.root.join("elsewhere/empty-target");
        mkdir(&target);
        mkdir(&base);
        let link = base.join("link");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .expect("mklink");
        assert!(made.status.success(), "could not create a junction");

        let out = p.scan(&base);

        assert!(out.findings.is_empty(), "{:?}", found(&out, &base));
        assert!(target.exists() && link.exists());
        // The junction counts as content, so `work` is not empty either.
        assert!(recheck_refusal(&base).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn an_access_denied_subtree_is_treated_as_not_empty() {
        let p = Profile::new();
        let base = p.root.join("work");
        let locked = base.join("locked");
        mkdir(&locked);
        mkdir(&base.join("plain"));
        let denied = std::process::Command::new("icacls")
            .arg(&locked)
            .args(["/deny", "*S-1-1-0:(RD)"])
            .output()
            .expect("icacls");
        if !denied.status.success() || fs::read_dir(&locked).is_ok() {
            // An elevated token bypasses the deny; nothing to assert here.
            return;
        }

        let out = p.scan(&base);

        let _ = std::process::Command::new("icacls")
            .arg(&locked)
            .args(["/remove:d", "*S-1-1-0"])
            .output();
        assert_eq!(found(&out, &base), vec!["plain"]);
        assert_eq!(out.unreadable.len(), 1);
    }

    #[test]
    fn a_protected_tree_is_skipped_and_counted() {
        let p = Profile::new();
        let base = p.root.join("work");
        mkdir(&base.join("win/inner"));
        mkdir(&base.join("free"));
        fs::write(base.join("keep.txt"), b"x").expect("file");
        let protected = ProtectedPaths::from_trees([base.join("win")]);
        let cancel = CancellationToken::new();

        let out = scan(
            &ScanContext {
                root: &base,
                protected: &protected,
                cancel: &cancel,
            },
            None,
        );

        assert_eq!(found(&out, &base), vec!["free"]);
        assert_eq!(out.skipped_protected, 1);
    }

    #[test]
    fn protected_paths_and_their_ancestors_are_do_not_delete() {
        let p = Profile::new();
        let base = p.root.join("work");
        mkdir(&base.join("holder/child"));
        let protected = ProtectedPaths::from_trees([base.join("holder/child")]);
        let brain = p.brain(protected);

        let mut holder = Finding::new(MODULE_ID, base.join("holder"), FindingKind::Directory);
        holder.meta = serde_json::json!({"nested": 1, "system": false});
        let mut inside =
            Finding::new(MODULE_ID, base.join("holder/child"), FindingKind::Directory);
        inside.meta = serde_json::json!({"nested": 0, "system": false});
        let recs = analyze(&[holder, inside], &brain);

        assert_eq!(recs.len(), 2);
        for rec in recs {
            assert_eq!(rec.risk, RiskBucket::DoNotDelete);
            assert_eq!(rec.action, Action::InfoOnly);
            assert!(!rec.reason.is_empty());
        }
    }

    #[test]
    fn a_system_folder_is_do_not_delete_with_a_reason() {
        let p = Profile::new();
        let brain = p.brain(ProtectedPaths::empty());
        let mut f = Finding::new(MODULE_ID, p.root.join("sys"), FindingKind::Directory);
        f.meta = serde_json::json!({"nested": 0, "system": true});

        let recs = analyze(&[f], &brain);

        assert_eq!(recs[0].risk, RiskBucket::DoNotDelete);
        assert!(recs[0].reason.contains("system folder"));
    }

    fn rate(p: &Profile, brain: &Brain, rel: &str, age_hours: u64) -> (RiskBucket, Action) {
        let path = p.root.join(rel);
        mkdir(&path);
        backdate_dir(&path, age_hours);
        let mut f = Finding::new(MODULE_ID, &path, FindingKind::Directory);
        f.modified = fs::symlink_metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(DateTime::<Utc>::from);
        f.meta = serde_json::json!({"nested": 0, "system": false});
        let rec = analyze(&[f], brain).remove(0);
        assert!(!rec.reason.is_empty());
        assert_eq!(rec.savings_bytes, 0);
        (rec.risk, rec.action)
    }

    #[test]
    fn the_rule_pack_buckets_follow_the_module_spec() {
        let p = Profile::new();
        let brain = p.brain(ProtectedPaths::empty());
        let old = 24 * 30;

        assert_eq!(
            rate(&p, &brain, "AppData/Local/Temp/old", 24 * 10).0,
            RiskBucket::Safe
        );
        assert_eq!(
            rate(&p, &brain, "AppData/Local/Temp/new", 1).0,
            RiskBucket::NeedsReview
        );
        assert_eq!(
            rate(&p, &brain, "AppData/Roaming/Vendor", old).0,
            RiskBucket::NeedsReview
        );
        assert_eq!(
            rate(&p, &brain, "AppData/Local/Vendor", old).0,
            RiskBucket::NeedsReview
        );
        assert_eq!(
            rate(&p, &brain, "ProgramData/Vendor", old).0,
            RiskBucket::DoNotDelete
        );
        assert_eq!(
            rate(&p, &brain, "Documents", old),
            (RiskBucket::DoNotDelete, Action::InfoOnly)
        );
        assert_eq!(
            rate(&p, &brain, "Desktop", old).0,
            RiskBucket::DoNotDelete
        );
        assert_eq!(
            rate(&p, &brain, "Documents/Taxes", old).0,
            RiskBucket::NeedsReview
        );
        assert_eq!(
            rate(&p, &brain, "Elsewhere/thing", old).0,
            RiskBucket::NeedsReview
        );
        // Never green outside temp, whatever the age.
        assert_ne!(
            rate(&p, &brain, "Documents/Taxes/2019", 24 * 900).0,
            RiskBucket::Safe
        );
    }

    #[test]
    fn a_folder_that_gained_a_file_is_refused_at_execute_time() {
        let p = Profile::new();
        let dir = p.root.join("work/a");
        mkdir(&dir);
        assert!(recheck_refusal(&dir).is_none());

        fs::write(dir.join("late.txt"), b"x").expect("file");

        assert!(recheck_refusal(&dir).is_some());
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
        rec: &Recommendation,
        finding: &Finding,
    ) -> crate::core::types::ExecReport {
        let bin = StubRecycleBin::default();
        let env = ExecEnvironment {
            quarantine: &rig.quarantine,
            db: &rig.conn,
            brain,
            recycle_bin: &bin,
            startup: &NoStartupControl,
            tweaks: &NoTweakControl,
            registry: &MapRegistry::new(),
        };
        let units = [ExecUnit {
            recommendation: rec,
            finding,
        }];
        execute(&units, &env, &ExecOptions::default(), None).expect("execute")
    }

    #[test]
    fn scan_quarantine_restore_puts_the_tree_back() {
        let p = Profile::new();
        let brain = p.brain(ProtectedPaths::empty());
        let base = p.root.join("AppData/Local/Temp");
        mkdir(&base.join("old/x/y"));
        backdate_dir(&base.join("old"), 24 * 10);
        fs::write(base.join("keep.tmp"), b"x").expect("file");

        let out = p.scan(&base);
        assert_eq!(found(&out, &base), vec!["old"]);
        let recs = analyze(&out.findings, &brain);
        assert_eq!(recs[0].action, Action::Quarantine, "{recs:?}");

        let rig = rig();
        let report = run_one(&rig, &brain, &recs[0], &out.findings[0]);

        assert_eq!(report.items[0].outcome, ExecOutcome::Succeeded, "{report:?}");
        assert!(!base.join("old").exists());
        let batch = report.batch_id.expect("batch");
        let (restored, failed) = rig
            .quarantine
            .restore_batch(&rig.conn, batch)
            .expect("restore");
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(restored.len(), 1);
        assert!(base.join("old/x/y").is_dir(), "the whole tree is back");
    }

    #[test]
    fn the_executor_skips_a_folder_that_is_no_longer_empty() {
        let p = Profile::new();
        let brain = p.brain(ProtectedPaths::empty());
        let base = p.root.join("AppData/Local/Temp");
        mkdir(&base.join("old"));
        backdate_dir(&base.join("old"), 24 * 10);
        let out = p.scan(&base);
        let recs = analyze(&out.findings, &brain);
        fs::write(base.join("old/new-file.txt"), b"x").expect("file");

        let report = run_one(&rig(), &brain, &recs[0], &out.findings[0]);

        assert!(
            matches!(report.items[0].outcome, ExecOutcome::Skipped { .. }),
            "{report:?}"
        );
        assert!(base.join("old/new-file.txt").exists(), "nothing was moved");
    }

    #[test]
    fn a_folder_finding_from_another_module_is_refused() {
        let p = Profile::new();
        let brain = p.brain(ProtectedPaths::empty());
        let base = p.root.join("AppData/Local/Temp");
        mkdir(&base.join("old/x"));
        backdate_dir(&base.join("old"), 24 * 10);
        let out = p.scan(&base);
        let recs = analyze(&out.findings, &brain);
        let mut finding = out.findings[0].clone();
        finding.module = "cleaner".to_owned();

        let report = run_one(&rig(), &brain, &recs[0], &finding);

        assert!(
            matches!(report.items[0].outcome, ExecOutcome::Skipped { .. }),
            "{report:?}"
        );
        assert!(base.join("old/x").is_dir(), "nothing was moved");
    }

    #[test]
    fn cancelling_stops_the_scan_with_no_findings() {
        let p = Profile::new();
        let base = p.root.join("work");
        mkdir(&base.join("a"));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let protected = ProtectedPaths::empty();

        let out = scan(
            &ScanContext {
                root: &base,
                protected: &protected,
                cancel: &cancel,
            },
            None,
        );

        assert!(out.cancelled);
        assert!(out.findings.is_empty());
    }
}

