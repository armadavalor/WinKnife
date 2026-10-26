//! Big-file archiver: find large files nobody has touched in a long time and,
//! on request, move them to a folder or drive the user picks (C16).
//!
//! # Scan
//!
//! Read-only. A file is reported when it is at least [`MIN_SIZE_BYTES`] and the
//! newer of its modified and accessed times is at least [`MIN_AGE_DAYS`] old. A
//! file whose clock cannot be read counts as new. Links are never followed and
//! protected trees are never reported.
//!
//! # Moving
//!
//! The move does not go through the executor; it is copy, verify, quarantine:
//!
//! 1. the source is re-checked against the protected-path gate,
//! 2. it is copied into the destination with `create_new` (nothing is ever
//!    overwritten) and flushed,
//! 3. the copy is verified by size and by [`full_hash`],
//! 4. only then is the original sent to quarantine,
//! 5. a JSON line is appended to [`MANIFEST_FILE_NAME`] in the destination.
//!
//! A failure at 2 or 3 removes only the copy this call just created; the source
//! is never touched. A failure at 4 leaves both the original and the verified
//! copy in place. The original stays restorable from the quarantine drawer for
//! as long as quarantine keeps it.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::brain::{Brain, ProtectedPaths};
use crate::core::content_hash::full_hash;
use crate::core::db::DbAccess;
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::path_compare::is_inside;
use crate::core::quarantine::Quarantine;
use crate::core::scanner::{self, CancellationToken};
use crate::core::types::{Finding, FindingKind, Recommendation, RiskBucket};

/// Module id, also the UI route.
pub const MODULE_ID: &str = "archiver";

/// Smallest file worth archiving: 1 GiB.
pub const MIN_SIZE_BYTES: u64 = 1024 * 1024 * 1024;

/// A file must have gone unused for at least this many days.
pub const MIN_AGE_DAYS: i64 = 365;

/// Name of the manifest written into every destination folder.
pub const MANIFEST_FILE_NAME: &str = "WinKnife-archive-manifest.jsonl";

/// Confidence reported for a gate verdict that is not a score.
const GATE_CONFIDENCE: u8 = 100;

/// Confidence of the "large and old" advice: both facts are measured.
const ADVICE_CONFIDENCE: u8 = 90;

const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[cfg(windows)]
const ATTR_REPARSE_POINT: u32 = 0x0000_0400;

/// The two limits a file has to clear. The commands always use
/// [`Thresholds::default`]; tests pass small values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// Minimum size in bytes.
    pub min_size_bytes: u64,
    /// Minimum days since last use.
    pub min_age_days: i64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            min_size_bytes: MIN_SIZE_BYTES,
            min_age_days: MIN_AGE_DAYS,
        }
    }
}

/// Extra facts attached to every finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiverMeta {
    /// The folder that was scanned; a destination may not sit inside it.
    pub root: PathBuf,
    /// Whole days since the file was last modified or opened.
    pub age_days: i64,
}

/// What a scan produced.
#[derive(Debug, Clone, Default)]
pub struct ScanOutput {
    /// Large, old files.
    pub findings: Vec<Finding>,
    /// Paths the walk could not read.
    pub unreadable: u64,
    /// Large, old files left out because they sit in a protected location.
    pub skipped_protected: u64,
    /// Entries looked at.
    pub entries_seen: u64,
    /// Whether the scan was cancelled part-way.
    pub cancelled: bool,
}

/// Progress callback: entries seen so far and the entry being looked at.
pub type ProgressSink<'a> = &'a mut dyn FnMut(u64, &Path);

/// Everything a scan needs.
pub struct ScanContext<'a> {
    /// Folder to scan.
    pub root: &'a Path,
    /// Locations never reported.
    pub protected: &'a ProtectedPaths,
    /// Cancellation.
    pub cancel: &'a CancellationToken,
    /// Size and age limits.
    pub thresholds: Thresholds,
    /// Reference instant for "how old".
    pub now: DateTime<Utc>,
}

/// Days since `file` was last used, when it clears both thresholds.
///
/// "Last used" is the newer of the two clocks; a file with neither counts as
/// new, so it is never offered.
pub fn cold_age_days(
    size_bytes: u64,
    modified: Option<DateTime<Utc>>,
    accessed: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    thresholds: Thresholds,
) -> Option<i64> {
    if size_bytes < thresholds.min_size_bytes {
        return None;
    }
    let last_used = modified.max(accessed)?;
    let age_days = (now - last_used).num_days();
    (age_days >= thresholds.min_age_days).then_some(age_days)
}

/// Scan `ctx.root` for large, old files. Read-only.
pub fn scan(ctx: &ScanContext<'_>, mut progress: Option<ProgressSink<'_>>) -> ScanOutput {
    let mut out = ScanOutput::default();
    let summary = scanner::walk(ctx.root, ctx.cancel, &mut |entry| {
        out.entries_seen += 1;
        if let Some(sink) = progress.as_deref_mut() {
            sink(out.entries_seen, &entry.path);
        }
        if entry.is_dir || entry.is_link {
            return;
        }
        let Some(age_days) = cold_age_days(
            entry.size_bytes,
            entry.modified,
            entry.accessed,
            ctx.now,
            ctx.thresholds,
        ) else {
            return;
        };
        if ctx.protected.reason_for(&entry.path).is_some() {
            out.skipped_protected += 1;
            return;
        }
        let mut finding = Finding::new(MODULE_ID, &entry.path, FindingKind::File);
        finding.size_bytes = entry.size_bytes;
        finding.created = entry.created;
        finding.modified = entry.modified;
        finding.accessed = entry.accessed;
        finding.meta = serde_json::to_value(ArchiverMeta {
            root: ctx.root.to_path_buf(),
            age_days,
        })
        .unwrap_or(serde_json::Value::Null);
        out.findings.push(finding);
    });
    out.unreadable = summary.unreadable.len() as u64;
    out.cancelled = summary.cancelled;
    out
}

/// Turn findings into explained recommendations. Read-only.
///
/// Every recommendation is `InfoOnly`: the move is its own command
/// ([`archive_one`]), not an executor action.
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

fn recommend(finding: &Finding, brain: &Brain) -> crate::core::error::Result<Recommendation> {
    if let Some(refusal) = brain.path_refusal(&finding.path) {
        return Recommendation::informational(
            vec![finding.id],
            RiskBucket::DoNotDelete,
            GATE_CONFIDENCE,
            refusal.reason,
            0,
        );
    }
    let age_days = finding
        .meta
        .get("ageDays")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(MIN_AGE_DAYS);
    let reason = format!(
        "{:.1} GB and not opened or changed for {age_days} days. Moving it to another drive frees \
         this space; the copy is checked byte for byte before the original goes to quarantine.",
        finding.size_bytes as f64 / BYTES_PER_GIB
    );
    Recommendation::informational(
        vec![finding.id],
        RiskBucket::NeedsReview,
        ADVICE_CONFIDENCE,
        reason,
        finding.size_bytes,
    )
}

/// Check the folder the user picked as an archive destination.
///
/// It must be an absolute path to an existing, real folder (not a link), not a
/// protected location, and not inside the scanned folder or the quarantine.
///
/// # Errors
/// A human-readable reason when the destination is refused.
pub fn validate_destination(
    dest: &Path,
    brain: &Brain,
    scan_root: &Path,
    quarantine_root: &Path,
) -> Result<PathBuf, String> {
    if !dest.is_absolute() {
        return Err(format!("`{}` is not a full path", dest.display()));
    }
    let metadata = fs::symlink_metadata(dest)
        .map_err(|err| format!("`{}` cannot be read: {err}", dest.display()))?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(format!(
            "`{}` is a link to somewhere else; pick the real folder",
            dest.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!("`{}` is not a folder", dest.display()));
    }
    if let Some(refusal) = brain.path_refusal(dest) {
        return Err(format!("`{}` is protected: {}", dest.display(), refusal.reason));
    }
    if is_inside(scan_root, dest) {
        return Err(format!(
            "`{}` is inside the folder that was scanned; pick a folder outside it",
            dest.display()
        ));
    }
    if is_inside(quarantine_root, dest) {
        return Err(format!(
            "`{}` is inside WinKnife's quarantine",
            dest.display()
        ));
    }
    Ok(dest.to_path_buf())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & ATTR_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

/// One line of the destination manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    /// When the move finished.
    pub archived_at: DateTime<Utc>,
    /// Where the file used to live.
    pub original_path: PathBuf,
    /// Where the verified copy lives now.
    pub archived_path: PathBuf,
    /// Size in bytes.
    pub size_bytes: u64,
    /// blake3 of the contents, shared by original and copy.
    pub hash: String,
    /// Quarantine item that holds the original.
    pub quarantine_item_id: Uuid,
    /// Quarantine batch of this move.
    pub batch_id: Uuid,
}

/// How one file fared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum ArchiveOutcome {
    /// Copied, verified, original quarantined.
    #[serde(rename_all = "camelCase")]
    Archived {
        /// The manifest line that was (or should have been) written.
        entry: ManifestEntry,
        /// Set when the manifest line could not be written.
        manifest_warning: Option<String>,
    },
    /// Not attempted; nothing changed.
    #[serde(rename_all = "camelCase")]
    Refused {
        /// Why.
        reason: String,
    },
    /// The copy failed or did not verify. The original is untouched; a
    /// half-made copy, if any, stays in the destination and the reason names it.
    #[serde(rename_all = "camelCase")]
    CopyFailed {
        /// Why.
        reason: String,
    },
    /// The copy verified but the original could not go to quarantine. Both
    /// files exist; nothing was lost.
    #[serde(rename_all = "camelCase")]
    QuarantineFailed {
        /// The verified copy.
        archived_path: PathBuf,
        /// Why.
        reason: String,
    },
}

/// Copy, verify and quarantine one file. See the module docs for the order.
pub fn archive_one(
    quarantine: &Quarantine,
    db: &dyn DbAccess,
    brain: &Brain,
    batch: Uuid,
    source: &Path,
    dest_dir: &Path,
) -> ArchiveOutcome {
    let refused = |reason: String| ArchiveOutcome::Refused { reason };
    if let Some(refusal) = brain.path_refusal(source) {
        return refused(refusal.reason);
    }
    match fs::symlink_metadata(source) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() && !is_reparse_point(&meta) => {}
        Ok(_) => return refused(format!("`{}` is no longer a plain file", source.display())),
        Err(err) => return refused(format!("`{}` cannot be read: {err}", source.display())),
    }
    let Some(name) = source.file_name() else {
        return refused(format!("`{}` has no file name", source.display()));
    };
    let target = dest_dir.join(name);
    if fs::symlink_metadata(&target).is_ok() {
        return refused(format!(
            "`{}` already exists; nothing is overwritten",
            target.display()
        ));
    }

    let (size_bytes, hash) = match copy_verified(source, &target) {
        Ok(facts) => facts,
        Err(reason) => return ArchiveOutcome::CopyFailed { reason },
    };

    let item = match quarantine.take_reporting(db, batch, source) {
        Ok(outcome) => outcome.item,
        Err(err) => {
            return ArchiveOutcome::QuarantineFailed {
                archived_path: target,
                reason: format!("the copy is safe, but the original could not go to quarantine: {err}"),
            }
        }
    };

    let entry = ManifestEntry {
        archived_at: Utc::now(),
        original_path: source.to_path_buf(),
        archived_path: target,
        size_bytes,
        hash,
        quarantine_item_id: item.id,
        batch_id: batch,
    };
    let manifest_warning = append_manifest(dest_dir, &entry)
        .err()
        .map(|err| format!("the archive manifest could not be updated: {err}"));
    ArchiveOutcome::Archived {
        entry,
        manifest_warning,
    }
}

/// Copy `source` to the new file `target` and prove they match. On any failure
/// the reason is returned. A half-made `target` is left where it is: only
/// quarantine may remove files (`no_other_module_removes_user_data`).
fn copy_verified(source: &Path, target: &Path) -> Result<(u64, String), String> {
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|err| format!("`{}` could not be created: {err}", target.display()))?;
    let result = (|| {
        let mut reader = fs::File::open(source)?;
        std::io::copy(&mut reader, &mut destination)?;
        destination.sync_all()?;
        drop(destination);
        let size = fs::metadata(source)?.len();
        let copied = fs::metadata(target)?.len();
        if size != copied {
            return Err(std::io::Error::other(format!(
                "the copy is {copied} bytes but the original is {size}"
            )));
        }
        let hash = full_hash(source)?;
        if full_hash(target)? != hash {
            return Err(std::io::Error::other("the copy's contents differ from the original"));
        }
        Ok((size, hash))
    })();
    result.map_err(|err| {
        if target.exists() {
            tracing::warn!(path = %target.display(), "a failed archive copy was left behind");
            format!(
                "copying `{}` failed: {err}. The incomplete copy `{}` was left in place; the original is untouched",
                source.display(),
                target.display()
            )
        } else {
            format!("copying `{}` failed: {err}", source.display())
        }
    })
}

fn append_manifest(dest_dir: &Path, entry: &ManifestEntry) -> std::io::Result<()> {
    let mut line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dest_dir.join(MANIFEST_FILE_NAME))?;
    file.write_all(line.as_bytes())?;
    file.sync_all()
}

/// Registry entry. The commands in `commands.rs` drive the scan and the move.
#[derive(Debug, Default)]
pub struct ArchiverModule;

impl ToolModule for ArchiverModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: MODULE_ID.to_owned(),
            name: "Big-File Archiver".to_owned(),
            description:
                "Find large files you have not used in a year and move them to another drive, verified."
                    .to_owned(),
            icon: "archive".to_owned(),
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
    use crate::core::brain::ProtectedPaths;
    use crate::core::db;
    use crate::core::platform::{AlwaysUnlocked, StubProcessProbe};
    use crate::core::quarantine::RetentionPolicy;
    use chrono::Duration;
    use std::sync::Arc;

    const SMALL: Thresholds = Thresholds {
        min_size_bytes: 10,
        min_age_days: 30,
    };

    struct Fixture {
        _dir: tempfile::TempDir,
        source_root: PathBuf,
        dest: PathBuf,
        protected_root: PathBuf,
        quarantine: Quarantine,
        brain: Brain,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let source_root = dir.path().join("data");
            let dest = dir.path().join("archive");
            let protected_root = dir.path().join("system");
            let quarantine_root = dir.path().join("quarantine");
            for path in [&source_root, &dest, &protected_root, &quarantine_root] {
                fs::create_dir_all(path).expect("dirs");
            }
            let protected =
                ProtectedPaths::from_trees([protected_root.clone(), quarantine_root.clone()]);
            let brain = Brain::builder()
                .protected_paths(protected)
                .lock_probe(Arc::new(AlwaysUnlocked))
                .process_probe(Arc::new(StubProcessProbe::none()))
                .build();
            Self {
                _dir: dir,
                source_root,
                dest,
                protected_root,
                quarantine: Quarantine::with_root(quarantine_root, RetentionPolicy::default()),
                brain,
            }
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.source_root.join(name);
            fs::write(&path, bytes).expect("write");
            path
        }

        fn archive(&self, source: &Path) -> ArchiveOutcome {
            let conn = db::open_in_memory().expect("db");
            let batch = self.quarantine.start_batch();
            archive_one(&self.quarantine, &conn, &self.brain, batch, source, &self.dest)
        }
    }

    fn days_ago(now: DateTime<Utc>, days: i64) -> Option<DateTime<Utc>> {
        Some(now - Duration::days(days))
    }

    #[test]
    fn threshold_filter_needs_both_size_and_age() {
        let now = Utc::now();
        assert_eq!(cold_age_days(10, days_ago(now, 40), None, now, SMALL), Some(40));
        assert_eq!(cold_age_days(9, days_ago(now, 40), None, now, SMALL), None, "too small");
        assert_eq!(cold_age_days(10, days_ago(now, 29), None, now, SMALL), None, "too new");
        assert_eq!(
            cold_age_days(10, days_ago(now, 400), days_ago(now, 5), now, SMALL),
            None,
            "opened recently"
        );
        assert_eq!(cold_age_days(10, None, None, now, SMALL), None, "no clock counts as new");
        assert_eq!(Thresholds::default().min_size_bytes, MIN_SIZE_BYTES);
        assert_eq!(Thresholds::default().min_age_days, MIN_AGE_DAYS);
    }

    #[test]
    fn scan_reports_only_large_old_files_with_a_reason() {
        let fx = Fixture::new();
        let big = fx.write("big.iso", &[7u8; 64]);
        fx.write("small.txt", b"hi");
        fs::write(fx.protected_root.join("big.bin"), [1u8; 64]).expect("write");
        let cancel = CancellationToken::new();
        // Every file was just written; a reference instant in the future makes them old.
        let ctx = ScanContext {
            root: fx.source_root.parent().expect("parent"),
            protected: fx.brain.protected_paths(),
            cancel: &cancel,
            thresholds: SMALL,
            now: Utc::now() + Duration::days(60),
        };
        let out = scan(&ctx, None);
        let paths: Vec<_> = out.findings.iter().map(|f| f.path.clone()).collect();
        assert_eq!(paths, vec![big]);
        assert_eq!(out.skipped_protected, 1);
        let recs = analyze(&out.findings, &fx.brain);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].risk, RiskBucket::NeedsReview);
        assert!(recs[0].reason.contains("days"), "{}", recs[0].reason);
    }

    #[test]
    fn archive_copies_verifies_then_quarantines() {
        let fx = Fixture::new();
        let source = fx.write("movie.mkv", b"the whole film");
        let outcome = fx.archive(&source);
        let ArchiveOutcome::Archived { entry, manifest_warning } = outcome else {
            panic!("expected archived, got {outcome:?}");
        };
        assert!(manifest_warning.is_none());
        assert!(!source.exists(), "original went to quarantine");
        assert_eq!(fs::read(&entry.archived_path).expect("copy"), b"the whole film");
        assert_eq!(entry.hash, full_hash(&entry.archived_path).expect("hash"));
        let quarantined: Vec<_> = walk_files(fx.quarantine.root());
        assert!(
            quarantined.iter().any(|p| fs::read(p).ok().as_deref() == Some(&b"the whole film"[..])),
            "original is held in quarantine: {quarantined:?}"
        );
        let manifest = fs::read_to_string(fx.dest.join(MANIFEST_FILE_NAME)).expect("manifest");
        let line: ManifestEntry = serde_json::from_str(manifest.trim()).expect("json line");
        assert_eq!(line, entry);
    }

    #[test]
    fn failed_copy_leaves_source_intact_and_no_copy() {
        let fx = Fixture::new();
        let source = fx.write("movie.mkv", b"the whole film");
        // A missing destination folder makes create_new fail.
        let conn = db::open_in_memory().expect("db");
        let missing = fx.dest.join("gone");
        let outcome = archive_one(
            &fx.quarantine,
            &conn,
            &fx.brain,
            fx.quarantine.start_batch(),
            &source,
            &missing,
        );
        assert!(matches!(outcome, ArchiveOutcome::CopyFailed { .. }), "{outcome:?}");
        assert_eq!(fs::read(&source).expect("source"), b"the whole film");
        assert!(walk_files(fx.quarantine.root()).is_empty());
    }

    #[test]
    fn a_copy_that_fails_after_creation_is_reported_not_removed() {
        let fx = Fixture::new();
        let target = fx.dest.join("a.bin");
        // The target gets created, then reading the vanished source fails.
        let err = copy_verified(&fx.source_root.join("vanished.bin"), &target)
            .expect_err("missing source");
        assert!(err.contains("vanished.bin"), "{err}");
        assert!(target.exists(), "only quarantine removes files");
        assert!(err.contains("incomplete copy"), "{err}");
    }

    #[test]
    fn existing_target_is_never_overwritten() {
        let fx = Fixture::new();
        let source = fx.write("a.bin", b"new");
        fs::write(fx.dest.join("a.bin"), b"keep me").expect("write");
        let outcome = fx.archive(&source);
        assert!(matches!(outcome, ArchiveOutcome::Refused { .. }), "{outcome:?}");
        assert_eq!(fs::read(fx.dest.join("a.bin")).expect("dest"), b"keep me");
        assert_eq!(fs::read(&source).expect("source"), b"new");
    }

    #[test]
    fn protected_source_is_refused() {
        let fx = Fixture::new();
        let source = fx.protected_root.join("big.bin");
        fs::write(&source, b"system data").expect("write");
        let outcome = fx.archive(&source);
        assert!(matches!(outcome, ArchiveOutcome::Refused { .. }), "{outcome:?}");
        assert!(source.exists());
        assert!(fs::read_dir(&fx.dest).expect("dest").next().is_none(), "nothing copied");

        let finding = Finding::new(MODULE_ID, &source, FindingKind::File);
        let recs = analyze(&[finding], &fx.brain);
        assert_eq!(recs[0].risk, RiskBucket::DoNotDelete);
    }

    #[test]
    fn destinations_are_validated() {
        let fx = Fixture::new();
        let ok = |dest: &Path| {
            validate_destination(dest, &fx.brain, &fx.source_root, fx.quarantine.root())
        };
        assert!(ok(&fx.dest).is_ok());
        assert!(ok(Path::new("relative/dir")).is_err(), "relative");
        assert!(ok(&fx.dest.join("missing")).is_err(), "missing");
        assert!(ok(&fx.protected_root).is_err(), "protected");
        assert!(ok(fx.quarantine.root()).is_err(), "quarantine");
        let inside = fx.source_root.join("sub");
        fs::create_dir(&inside).expect("dir");
        assert!(ok(&inside).is_err(), "inside the scanned folder");
        assert!(ok(&fx.source_root).is_err(), "the scanned folder itself");
        let file = fx.write("f.txt", b"x");
        assert!(ok(&file).is_err(), "a file");
    }

    fn walk_files(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let cancel = CancellationToken::new();
        scanner::walk(root, &cancel, &mut |entry| {
            if !entry.is_dir {
                out.push(entry.path.clone());
            }
        });
        out
    }
}
