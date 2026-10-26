//! Duplicate folders (research C3).
//!
//! Two folders are duplicates when they hold the same files under the same
//! relative names with the same bytes. The group key is a blake3 over the sorted
//! `(relative path, content hash)` pairs, so it is also stable across scans and
//! doubles as the key of the "ignore this group" list.
//!
//! Findings stay per *file*: the executor's gate 3 refuses directories, and gate
//! 4 re-hashes each copy against the same file in the kept folder. Moving every
//! file of a duplicate folder leaves its (now empty) skeleton behind; the empty
//! folder module can tidy that up.
//!
//! A folder is never grouped when anything inside it is protected, a reparse
//! point, or unreadable: the user would be agreeing to a folder whose full
//! contents nobody looked at. Hidden and system files (`desktop.ini`,
//! `Thumbs.db`) are left out of the comparison and never offered.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use rayon::prelude::*;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    is_hidden_or_system, plural, resolve_root, DuplicateContext, DuplicateProgress, ProgressSink,
    Stage, DUPLICATE_CONFIDENCE,
};
use crate::core::brain::Brain;
use crate::core::content_hash::full_hash_cancellable;
use crate::core::error::Result;
use crate::core::path_compare;
use crate::core::scanner::{self, WalkOptions};
use crate::core::settings;
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket,
};

/// Settings key of the ignored group ids (`Vec<String>`).
pub const IGNORED_KEY: &str = "duplicates.ignoredFolderGroups";

/// Which folder in a group stays put.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "folder", rename_all = "camelCase")]
pub enum KeepRule {
    /// The folder created first (falls back to modified when created is unknown).
    #[default]
    Oldest,
    /// The folder created last.
    Newest,
    /// The member at, or inside, a folder the user picked.
    PreferredFolder(PathBuf),
}

/// What a folder scan found.
#[derive(Debug, Default)]
pub struct FolderScan {
    /// One `File` finding per file per duplicate folder.
    pub findings: Vec<Finding>,
    /// Roots refused before walking, with the reason.
    pub skipped_roots: Vec<String>,
    /// Entries the walk could not read.
    pub unreadable: u64,
    /// The scan was cancelled; `findings` is empty.
    pub cancelled: bool,
}

/// One group of identical folders, as the UI shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderGroup {
    /// Content signature; the key of the ignore list.
    pub id: String,
    /// Bytes in one copy.
    pub size_bytes: u64,
    /// Files in one copy.
    pub file_count: usize,
    /// Bytes held by every copy but one.
    pub wasted_bytes: u64,
    /// The rule that picked the keeper.
    pub keep_rule: KeepRule,
    /// The folders, keeper first.
    pub members: Vec<FolderMember>,
}

/// One folder of a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderMember {
    pub path: PathBuf,
    pub created: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
    /// This folder stays put.
    pub keeper: bool,
    /// A protected location: never offered.
    pub protected: bool,
    /// Why this folder is kept, offered, or left alone.
    pub reason: String,
    pub finding_ids: Vec<Uuid>,
    pub recommendation_ids: Vec<Uuid>,
}

/* -------------------------------------------------------------------------
 * scan
 * ---------------------------------------------------------------------- */

#[derive(Debug, Clone, Copy, Default)]
struct Stamps {
    created: Option<DateTime<Utc>>,
    modified: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct Entry {
    rel: String,
    size: u64,
    path: PathBuf,
}

#[derive(Debug, Default)]
struct Tree {
    dirs: HashMap<PathBuf, Stamps>,
    /// Folder -> every file under it, sorted by relative path.
    listings: HashMap<PathBuf, Vec<Entry>>,
}

/// Find groups of identical folders under the context's roots. Read-only.
pub fn scan_folders(
    ctx: &DuplicateContext<'_>,
    ignored: &HashSet<String>,
    progress: Option<ProgressSink<'_>>,
) -> FolderScan {
    let mut output = FolderScan::default();
    let min_size = ctx.settings.clone().validated().min_size_bytes;

    let mut tree = Tree::default();
    for root in distinct_roots(ctx.roots) {
        if let Err(why) = resolve_root(&root, ctx.protected) {
            output.skipped_roots.push(why);
            continue;
        }
        if let Some(report) = progress {
            report(&DuplicateProgress {
                stage: Stage::Walking,
                done: 0,
                total: 0,
                current_path: root.clone(),
            });
        }
        let (walked, unreadable, cancelled) = walk_tree(&root, ctx);
        output.unreadable += unreadable;
        if cancelled {
            return FolderScan {
                cancelled: true,
                ..output
            };
        }
        tree.dirs.extend(walked.dirs);
        tree.listings.extend(walked.listings);
    }

    let candidates = size_candidates(&tree, min_size);
    let Some((hashes, unhashable)) = hash_files(&candidates, ctx, progress) else {
        return FolderScan {
            cancelled: true,
            ..output
        };
    };
    output.unreadable += unhashable;
    let groups = content_groups(&candidates, &hashes, ignored);
    output.findings = emit_findings(&groups, &tree, &hashes);
    output
}

/// Drop repeated and nested roots: a folder must be walked once.
pub(super) fn distinct_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter(|root| {
            !roots.iter().any(|other| {
                !std::ptr::eq(*root, other) && path_compare::is_inside(other, root)
            })
        })
        .fold(Vec::new(), |mut kept: Vec<PathBuf>, root| {
            if !kept.iter().any(|k| path_compare::same_path(k, root)) {
                kept.push(root.clone());
            }
            kept
        })
}

fn walk_tree(root: &Path, ctx: &DuplicateContext<'_>) -> (Tree, u64, bool) {
    let mut dirs = HashMap::new();
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    let mut poisoned: HashSet<PathBuf> = HashSet::new();
    dirs.insert(root.to_path_buf(), stamps_of(root));

    let options = WalkOptions {
        report_links: true,
        ..Default::default()
    };
    let summary = scanner::walk_with(root, options, ctx.cancel, &mut |entry| {
        if entry.is_link || ctx.protected.reason_for(&entry.path).is_some() {
            poison(root, &entry.path, &mut poisoned);
        } else if entry.is_dir {
            dirs.insert(
                entry.path.clone(),
                Stamps {
                    created: entry.created,
                    modified: entry.modified,
                },
            );
        } else if !is_hidden_or_system(&entry.path) {
            files.push((entry.path.clone(), entry.size_bytes));
        }
    });
    for path in &summary.unreadable {
        poison(root, path, &mut poisoned);
    }

    let mut listings: HashMap<PathBuf, Vec<Entry>> = HashMap::new();
    for (path, size) in files {
        for dir in path.ancestors().skip(1) {
            if !dir.starts_with(root) {
                break;
            }
            if !poisoned.contains(dir) {
                let rel = path.strip_prefix(dir).unwrap_or(&path);
                listings.entry(dir.to_path_buf()).or_default().push(Entry {
                    rel: rel.to_string_lossy().into_owned(),
                    size,
                    path: path.clone(),
                });
            }
        }
    }
    for entries in listings.values_mut() {
        entries.sort_by(|a, b| a.rel.cmp(&b.rel));
    }

    let unreadable = summary.unreadable.len() as u64;
    (Tree { dirs, listings }, unreadable, summary.cancelled)
}

/// Mark `path` and every folder above it, up to `root`, as not groupable.
fn poison(root: &Path, path: &Path, poisoned: &mut HashSet<PathBuf>) {
    for dir in path.ancestors() {
        if !dir.starts_with(root) {
            break;
        }
        poisoned.insert(dir.to_path_buf());
    }
}

fn stamps_of(path: &Path) -> Stamps {
    let metadata = std::fs::metadata(path).ok();
    let utc = |time: Option<std::time::SystemTime>| time.map(DateTime::<Utc>::from);
    Stamps {
        created: utc(metadata.as_ref().and_then(|m| m.created().ok())),
        modified: utc(metadata.as_ref().and_then(|m| m.modified().ok())),
    }
}

fn signature(entries: &[Entry], value: impl Fn(&Entry) -> String) -> String {
    let mut hasher = blake3::Hasher::new();
    for entry in entries {
        hasher.update(entry.rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(value(entry).as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

/// Folders big enough to matter whose names-and-sizes match another folder's.
fn size_candidates(tree: &Tree, min_size: u64) -> Vec<(PathBuf, Vec<Entry>)> {
    let mut by_size: HashMap<String, Vec<(PathBuf, Vec<Entry>)>> = HashMap::new();
    for (dir, entries) in &tree.listings {
        let total: u64 = entries.iter().map(|e| e.size).sum();
        if entries.is_empty() || total < min_size {
            continue;
        }
        let key = signature(entries, |e| e.size.to_string());
        by_size
            .entry(key)
            .or_default()
            .push((dir.clone(), entries.clone()));
    }
    by_size
        .into_values()
        .filter(|members| members.len() >= 2)
        .flatten()
        .collect()
}

/// Full hash of every candidate file once, plus how many could not be read.
/// `None` when cancelled.
fn hash_files(
    candidates: &[(PathBuf, Vec<Entry>)],
    ctx: &DuplicateContext<'_>,
    progress: Option<ProgressSink<'_>>,
) -> Option<(HashMap<PathBuf, String>, u64)> {
    let paths: Vec<&PathBuf> = candidates
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|e| &e.path))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let total = paths.len() as u64;
    let done = AtomicU64::new(0);
    let unreadable = AtomicU64::new(0);
    let is_cancelled = || ctx.cancel.is_cancelled();

    let hashes: HashMap<PathBuf, String> = paths
        .par_iter()
        .filter_map(|path| {
            if is_cancelled() {
                return None;
            }
            let hash = match full_hash_cancellable(path, &is_cancelled) {
                Ok(hash) => hash,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => return None,
                Err(err) => {
                    unreadable.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(
                        path = %path.display(),
                        error = %err,
                        "folder member skipped: could not be read in full"
                    );
                    return None;
                }
            };
            let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(report) = progress {
                report(&DuplicateProgress {
                    stage: Stage::FullHashing,
                    done: finished,
                    total,
                    current_path: (*path).clone(),
                });
            }
            Some(((*path).clone(), hash))
        })
        .collect();

    (!ctx.cancel.is_cancelled()).then(|| (hashes, unreadable.load(Ordering::Relaxed)))
}

/// Group id -> member folders, outermost duplicates only, ignored groups out.
fn content_groups(
    candidates: &[(PathBuf, Vec<Entry>)],
    hashes: &HashMap<PathBuf, String>,
    ignored: &HashSet<String>,
) -> BTreeMap<String, Vec<(PathBuf, Vec<Entry>)>> {
    let mut groups: BTreeMap<String, Vec<(PathBuf, Vec<Entry>)>> = BTreeMap::new();
    for (dir, entries) in candidates {
        // A file that could not be hashed leaves its folders unproven.
        if entries.iter().any(|e| !hashes.contains_key(&e.path)) {
            continue;
        }
        let key = signature(entries, |e| hashes[&e.path].clone());
        groups
            .entry(key)
            .or_default()
            .push((dir.clone(), entries.clone()));
    }
    groups.retain(|_, members| members.len() >= 2);

    // shortcut: loses intra-folder dup subfolders when the parent is also duplicated, upgrade if users report it
    let members: HashSet<PathBuf> = groups
        .values()
        .flatten()
        .map(|(dir, _)| dir.clone())
        .collect();
    let has_member_above = |dir: &Path| dir.ancestors().skip(1).any(|a| members.contains(a));
    groups
        .into_iter()
        .filter(|(id, _)| !ignored.contains(id))
        .map(|(id, list)| {
            let outer: Vec<_> = list
                .into_iter()
                .filter(|(dir, _)| !has_member_above(dir))
                .collect();
            (id, outer)
        })
        .filter(|(_, list)| list.len() >= 2)
        .collect()
}

fn emit_findings(
    groups: &BTreeMap<String, Vec<(PathBuf, Vec<Entry>)>>,
    tree: &Tree,
    hashes: &HashMap<PathBuf, String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (id, members) in groups {
        for (dir, entries) in members {
            let stamps = tree.dirs.get(dir).copied().unwrap_or_default();
            for entry in entries {
                let mut finding = Finding::new("duplicates", entry.path.clone(), FindingKind::File);
                finding.size_bytes = entry.size;
                finding.meta = serde_json::json!({
                    "folderGroupId": id,
                    "folderPath": dir,
                    "folderCreated": stamps.created,
                    "folderModified": stamps.modified,
                    "relPath": entry.rel,
                    "contentHash": hashes[&entry.path],
                });
                findings.push(finding);
            }
        }
    }
    findings
}

/* -------------------------------------------------------------------------
 * keep rule
 * ---------------------------------------------------------------------- */

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileMeta {
    folder_group_id: String,
    folder_path: PathBuf,
    folder_created: Option<DateTime<Utc>>,
    folder_modified: Option<DateTime<Utc>>,
    rel_path: String,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct FileRef {
    finding_id: Uuid,
    path: PathBuf,
    rel: String,
    hash: String,
    size: u64,
}

#[derive(Debug, Clone)]
struct Draft {
    path: PathBuf,
    stamps: Stamps,
    files: Vec<FileRef>,
}

impl Draft {
    fn age(&self) -> Option<DateTime<Utc>> {
        self.stamps.created.or(self.stamps.modified)
    }
}

/// Index of the one member that stays, and why. Always exactly one index.
fn pick_keeper(members: &[Draft], rule: &KeepRule) -> (usize, String) {
    let oldest = || {
        members
            .iter()
            .enumerate()
            .min_by_key(|(_, m)| (m.age().is_none(), m.age(), m.path.clone()))
            .map_or(0, |(i, _)| i)
    };
    match rule {
        KeepRule::Oldest => (oldest(), "Kept because it is the oldest copy.".to_owned()),
        KeepRule::Newest => {
            let index = members
                .iter()
                .enumerate()
                .min_by_key(|(_, m)| (std::cmp::Reverse(m.age()), m.path.clone()))
                .map_or(0, |(i, _)| i);
            (index, "Kept because it is the newest copy.".to_owned())
        }
        KeepRule::PreferredFolder(folder) => {
            let preferred = members.iter().position(|m| {
                path_compare::same_path(folder, &m.path) || path_compare::is_inside(folder, &m.path)
            });
            match preferred {
                Some(index) => (
                    index,
                    format!("Kept because it is in your preferred folder `{}`.", folder.display()),
                ),
                None => (
                    oldest(),
                    format!(
                        "Kept because it is the oldest copy; your preferred folder `{}` holds none of these.",
                        folder.display()
                    ),
                ),
            }
        }
    }
}

/* -------------------------------------------------------------------------
 * analyze
 * ---------------------------------------------------------------------- */

#[derive(Debug, Clone)]
struct Planned {
    member: usize,
    file: FileRef,
    risk: RiskBucket,
    action: Action,
    reason: String,
}

/// Turn folder findings into groups and per-file recommendations. Never Safe.
pub fn analyze(
    findings: &[Finding],
    brain: &Brain,
    rules: &HashMap<String, KeepRule>,
) -> (Vec<FolderGroup>, Vec<Recommendation>) {
    let mut drafts: BTreeMap<String, BTreeMap<PathBuf, Draft>> = BTreeMap::new();
    for finding in findings {
        let Ok(meta) = serde_json::from_value::<FileMeta>(finding.meta.clone()) else {
            continue;
        };
        let draft = drafts
            .entry(meta.folder_group_id)
            .or_default()
            .entry(meta.folder_path.clone())
            .or_insert_with(|| Draft {
                path: meta.folder_path,
                stamps: Stamps {
                    created: meta.folder_created,
                    modified: meta.folder_modified,
                },
                files: Vec::new(),
            });
        draft.files.push(FileRef {
            finding_id: finding.id,
            path: finding.path.clone(),
            rel: meta.rel_path,
            hash: meta.content_hash,
            size: finding.size_bytes,
        });
    }

    let mut groups = Vec::new();
    let mut recommendations = Vec::new();
    for (id, members) in drafts {
        let members: Vec<Draft> = members.into_values().collect();
        if members.len() < 2 {
            continue;
        }
        let rule = rules.get(&id).cloned().unwrap_or_default();
        let (group, recs) = analyze_group(id, members, brain, rule);
        groups.push(group);
        recommendations.extend(recs);
    }
    groups.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes).then(a.id.cmp(&b.id)));
    (groups, recommendations)
}

fn analyze_group(
    id: String,
    members: Vec<Draft>,
    brain: &Brain,
    rule: KeepRule,
) -> (FolderGroup, Vec<Recommendation>) {
    let (keeper, keep_reason) = pick_keeper(&members, &rule);
    let keeper_path = members[keeper].path.clone();
    let refusals: Vec<Option<String>> = members
        .iter()
        .map(|m| brain.path_refusal(&m.path).map(|r| r.reason))
        .collect();

    let others = plural(members.len() - 1, "folder holds", "folders hold");
    let plan: Vec<Planned> = members
        .iter()
        .enumerate()
        .flat_map(|(index, member)| {
            member.files.iter().map(move |file| (index, file))
        })
        .map(|(index, file)| {
            let (risk, action, reason) = if let Some(why) = refusals[index]
                .clone()
                .or_else(|| brain.path_refusal(&file.path).map(|r| r.reason))
            {
                (RiskBucket::DoNotDelete, Action::InfoOnly, why)
            } else if !brain.lock_state(&file.path).is_free() {
                let lock = brain.lock_state(&file.path);
                (RiskBucket::DoNotDelete, Action::InfoOnly, format!("left alone: {}", lock.reason()))
            } else if index == keeper {
                (
                    RiskBucket::NeedsReview,
                    Action::InfoOnly,
                    format!("{keep_reason} The other {others} the same files."),
                )
            } else {
                (
                    RiskBucket::NeedsReview,
                    Action::Quarantine,
                    format!(
                        "Its folder is identical to `{}`, which is kept; the same file there is `{}`. \
                         It goes to quarantine and can be restored.",
                        keeper_path.display(),
                        keeper_path.join(&file.rel).display()
                    ),
                )
            };
            Planned { member: index, file: file.clone(), risk, action, reason }
        })
        .collect();
    let plan = keep_at_least_one(plan);

    let recommendations: Vec<Recommendation> = plan
        .iter()
        .filter_map(|planned| to_recommendation(planned, keeper, &keeper_path))
        .collect();
    let group = build_group(id, rule, &members, keeper, &keep_reason, &refusals, &recommendations);
    (group, recommendations)
}

/// Never let every copy of a file go: if all copies of one relative path would
/// be quarantined, none is.
fn keep_at_least_one(plan: Vec<Planned>) -> Vec<Planned> {
    let mut kept_somewhere: HashSet<String> = HashSet::new();
    for planned in &plan {
        if !planned.action.is_mutating() {
            kept_somewhere.insert(planned.file.rel.clone());
        }
    }
    plan.into_iter()
        .map(|planned| {
            if kept_somewhere.contains(&planned.file.rel) {
                planned
            } else {
                Planned {
                    risk: RiskBucket::DoNotDelete,
                    action: Action::InfoOnly,
                    reason: "Left alone: every copy of this file would have been quarantined, \
                             and one copy always stays."
                        .to_owned(),
                    ..planned
                }
            }
        })
        .collect()
}

fn to_recommendation(planned: &Planned, keeper: usize, keeper_path: &Path) -> Option<Recommendation> {
    let justification = Justification::DuplicateOf {
        content_hash: planned.file.hash.clone(),
        keeper: planned.member == keeper,
        keeper_path: Some(keeper_path.join(&planned.file.rel)),
    };
    let savings = if planned.action.is_mutating() {
        planned.file.size
    } else {
        0
    };
    Recommendation::new(
        vec![planned.file.finding_id],
        planned.risk,
        DUPLICATE_CONFIDENCE,
        planned.action,
        planned.reason.clone(),
        savings,
        justification,
    )
    .map_err(|err| {
        tracing::error!(
            path = %planned.file.path.display(),
            error = %err,
            "dropped a folder duplicate recommendation that failed its own invariants"
        );
    })
    .ok()
}

fn build_group(
    id: String,
    keep_rule: KeepRule,
    members: &[Draft],
    keeper: usize,
    keep_reason: &str,
    refusals: &[Option<String>],
    recommendations: &[Recommendation],
) -> FolderGroup {
    let size_bytes: u64 = members[keeper].files.iter().map(|f| f.size).sum();
    let file_count = members[keeper].files.len();
    let mut out: Vec<FolderMember> = members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let finding_ids: Vec<Uuid> = member.files.iter().map(|f| f.finding_id).collect();
            let reason = match (&refusals[index], index == keeper) {
                (Some(why), _) => why.clone(),
                (None, true) => keep_reason.to_owned(),
                (None, false) => format!(
                    "Identical to `{}`; its files go to quarantine and can be restored.",
                    members[keeper].path.display()
                ),
            };
            FolderMember {
                path: member.path.clone(),
                created: member.stamps.created,
                modified: member.stamps.modified,
                keeper: index == keeper,
                protected: refusals[index].is_some(),
                reason,
                recommendation_ids: recommendations
                    .iter()
                    .filter(|r| r.finding_ids.iter().any(|f| finding_ids.contains(f)))
                    .map(|r| r.id)
                    .collect(),
                finding_ids,
            }
        })
        .collect();
    out.sort_by_key(|m| !m.keeper);
    FolderGroup {
        id,
        size_bytes,
        file_count,
        wasted_bytes: size_bytes.saturating_mul(members.len() as u64 - 1),
        keep_rule,
        members: out,
    }
}

/* -------------------------------------------------------------------------
 * ignore list
 * ---------------------------------------------------------------------- */

/// Group ids the user asked never to see again.
pub fn ignored_groups(conn: &Connection) -> Result<HashSet<String>> {
    Ok(settings::get_json::<Vec<String>>(conn, IGNORED_KEY)?
        .unwrap_or_default()
        .into_iter()
        .collect())
}

/// Add one group id to the ignore list.
pub fn ignore_group(conn: &Connection, id: &str) -> Result<()> {
    let mut ids = settings::get_json::<Vec<String>>(conn, IGNORED_KEY)?.unwrap_or_default();
    if !ids.iter().any(|known| known == id) {
        ids.push(id.to_owned());
    }
    settings::set_json(conn, IGNORED_KEY, &ids)
}

/// Forget every ignored group.
pub fn clear_ignored(conn: &Connection) -> Result<()> {
    settings::clear(conn, IGNORED_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::brain::ProtectedPaths;
    use crate::core::db;
    use crate::core::platform::{AlwaysUnlocked, StubProcessProbe};
    use crate::core::scanner::CancellationToken;
    use crate::modules::duplicates::DuplicatesSettings;
    use std::fs;
    use std::sync::Arc;

    const MIN: u64 = 1;

    /// Writes `bytes` after a fixed pad, so every file clears the 4 KiB size floor.
    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let mut padded = vec![b'.'; 5000];
        padded.extend_from_slice(bytes);
        fs::write(path, padded).expect("write");
    }

    /// A folder with two files and a nested subfolder.
    fn album(dir: &Path, tweak: &[u8]) {
        write(&dir.join("a.jpg"), b"aaaa-photo");
        write(&dir.join("b.jpg"), tweak);
        write(&dir.join("sub").join("c.jpg"), b"cccc-photo");
    }

    fn scan_with(root: &Path, protected: &ProtectedPaths, ignored: &HashSet<String>) -> FolderScan {
        let settings = DuplicatesSettings {
            min_size_bytes: MIN,
            roots: Vec::new(),
        };
        let cancel = CancellationToken::new();
        scan_folders(
            &DuplicateContext {
                roots: std::slice::from_ref(&root.to_path_buf()),
                protected,
                settings: &settings,
                cancel: &cancel,
            },
            ignored,
            None,
        )
    }

    fn scan(root: &Path) -> FolderScan {
        scan_with(root, &ProtectedPaths::empty(), &HashSet::new())
    }

    fn brain_with(protected: ProtectedPaths) -> Brain {
        Brain::builder()
            .lock_probe(Arc::new(AlwaysUnlocked))
            .process_probe(Arc::new(StubProcessProbe::none()))
            .protected_paths(protected)
            .build()
    }

    fn folders_of(scan: &FolderScan) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = scan
            .findings
            .iter()
            .filter_map(|f| f.meta["folderPath"].as_str().map(PathBuf::from))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        dirs.sort();
        dirs
    }

    #[test]
    fn identical_folders_with_nested_subfolders_form_one_group_of_the_outer_folders() {
        let dir = tempfile::tempdir().expect("tempdir");
        album(&dir.path().join("Photos"), b"bbbb-photo");
        album(&dir.path().join("Backup").join("Photos copy"), b"bbbb-photo");

        let found = scan(dir.path());

        // `sub` inside each copy is also identical, but only the outer pair is offered.
        assert_eq!(
            folders_of(&found),
            vec![
                dir.path().join("Backup").join("Photos copy"),
                dir.path().join("Photos")
            ]
        );
        assert_eq!(found.findings.len(), 6);
    }

    #[test]
    fn a_differing_size_or_content_breaks_the_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        album(&dir.path().join("One"), b"bbbb-photo");
        album(&dir.path().join("Two"), b"bbbb-photo-longer");
        album(&dir.path().join("Three"), b"BBBB-photo");

        let found = scan(dir.path());

        // Only the identical `sub` folders are left as a group.
        assert!(folders_of(&found).iter().all(|p| p.ends_with("sub")));
    }

    #[test]
    fn a_file_that_cannot_be_hashed_is_counted_as_unreadable() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        album(&dir.path().join("One"), b"bbbb-photo");
        album(&dir.path().join("Two"), b"bbbb-photo");
        // Exclusive handle: the walk still lists it, the hash open fails.
        let _hold = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(dir.path().join("Two").join("a.jpg"))
            .expect("hold");

        let found = scan(dir.path());

        assert_eq!(found.unreadable, 1, "{found:?}");
        assert!(!found.cancelled);
    }

    #[test]
    fn a_renamed_file_breaks_the_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("One").join("a.txt"), b"same");
        write(&dir.path().join("Two").join("renamed.txt"), b"same");

        assert!(scan(dir.path()).findings.is_empty());
    }

    #[test]
    fn the_scan_changes_nothing_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        album(&dir.path().join("One"), b"bbbb");
        album(&dir.path().join("Two"), b"bbbb");
        let snapshot = || {
            let mut all = Vec::new();
            scanner::walk(dir.path(), &CancellationToken::new(), &mut |e| {
                all.push((e.path.clone(), e.size_bytes, e.modified));
            });
            all
        };
        let before = snapshot();
        let _ = scan(dir.path());
        assert_eq!(before, snapshot());
    }

    fn draft(path: &str, created: Option<i64>) -> Draft {
        Draft {
            path: PathBuf::from(path),
            stamps: Stamps {
                created: created.and_then(|s| DateTime::<Utc>::from_timestamp(s, 0)),
                modified: None,
            },
            files: Vec::new(),
        }
    }

    #[test]
    fn keep_rules_pick_the_right_copy() {
        let members = vec![
            draft(r"C:\b\new", Some(300)),
            draft(r"C:\a\old", Some(100)),
            draft(r"C:\c\unknown", None),
        ];
        assert_eq!(pick_keeper(&members, &KeepRule::Oldest).0, 1);
        assert_eq!(pick_keeper(&members, &KeepRule::Newest).0, 0);
        let preferred = KeepRule::PreferredFolder(PathBuf::from(r"C:\c"));
        assert_eq!(pick_keeper(&members, &preferred).0, 2);

        let elsewhere = KeepRule::PreferredFolder(PathBuf::from(r"D:\nowhere"));
        let (index, reason) = pick_keeper(&members, &elsewhere);
        assert_eq!(index, 1, "falls back to the oldest");
        assert!(reason.contains("holds none"), "{reason}");
    }

    #[test]
    fn exactly_one_folder_is_kept_and_its_files_are_never_quarantined() {
        let dir = tempfile::tempdir().expect("tempdir");
        album(&dir.path().join("One"), b"bbbb");
        album(&dir.path().join("Two"), b"bbbb");
        album(&dir.path().join("Three"), b"bbbb");
        let found = scan(dir.path());
        let brain = brain_with(ProtectedPaths::empty());

        for rule in [
            KeepRule::Oldest,
            KeepRule::Newest,
            KeepRule::PreferredFolder(dir.path().join("Two")),
        ] {
            let id = found.findings[0].meta["folderGroupId"].as_str().unwrap().to_owned();
            let rules = HashMap::from([(id, rule.clone())]);
            let (groups, recs) = analyze(&found.findings, &brain, &rules);
            assert_eq!(groups.len(), 1);
            let group = &groups[0];
            assert_eq!(group.members.iter().filter(|m| m.keeper).count(), 1);
            let keeper = group.members.iter().find(|m| m.keeper).unwrap();
            if let KeepRule::PreferredFolder(folder) = &rule {
                assert_eq!(&keeper.path, folder);
            }
            for rec in &recs {
                assert_ne!(rec.risk, RiskBucket::Safe);
                if keeper.recommendation_ids.contains(&rec.id) {
                    assert_eq!(rec.action, Action::InfoOnly);
                } else {
                    assert_eq!(rec.action, Action::Quarantine, "{}", rec.reason);
                }
                assert!(!rec.reason.trim().is_empty());
            }
            assert_eq!(recs.iter().filter(|r| r.action == Action::Quarantine).count(), 6);
        }
    }

    #[test]
    fn the_guard_never_lets_every_copy_be_quarantined() {
        let file = |rel: &str| FileRef {
            finding_id: Uuid::new_v4(),
            path: PathBuf::from(rel),
            rel: rel.to_owned(),
            hash: "h".to_owned(),
            size: 1,
        };
        let planned = |member, rel: &str, action| Planned {
            member,
            file: file(rel),
            risk: RiskBucket::NeedsReview,
            action,
            reason: "x".to_owned(),
        };
        let plan = vec![
            planned(0, "a", Action::Quarantine),
            planned(1, "a", Action::Quarantine),
            planned(0, "b", Action::InfoOnly),
            planned(1, "b", Action::Quarantine),
        ];

        let guarded = keep_at_least_one(plan);

        let action = |m: usize, rel: &str| {
            guarded
                .iter()
                .find(|p| p.member == m && p.file.rel == rel)
                .map(|p| (p.risk, p.action))
                .unwrap()
        };
        assert_eq!(action(0, "a"), (RiskBucket::DoNotDelete, Action::InfoOnly));
        assert_eq!(action(1, "a"), (RiskBucket::DoNotDelete, Action::InfoOnly));
        assert_eq!(action(1, "b"), (RiskBucket::NeedsReview, Action::Quarantine));
    }

    #[test]
    fn a_protected_member_is_never_selected() {
        let dir = tempfile::tempdir().expect("tempdir");
        album(&dir.path().join("Mine"), b"bbbb");
        album(&dir.path().join("Guarded"), b"bbbb");
        let found = scan(dir.path());
        assert_eq!(folders_of(&found).len(), 2);

        // Protected at analyze time (e.g. the list changed since the scan).
        let brain = brain_with(ProtectedPaths::from_trees([dir.path().join("Guarded")]));
        for rule in [KeepRule::Oldest, KeepRule::Newest] {
            let id = found.findings[0].meta["folderGroupId"].as_str().unwrap().to_owned();
            let (groups, recs) = analyze(&found.findings, &brain, &HashMap::from([(id, rule)]));
            let guarded = groups[0]
                .members
                .iter()
                .find(|m| m.path.ends_with("Guarded"))
                .unwrap();
            assert!(guarded.protected);
            for rec in recs.iter().filter(|r| guarded.recommendation_ids.contains(&r.id)) {
                assert_eq!(rec.risk, RiskBucket::DoNotDelete);
                assert_eq!(rec.action, Action::InfoOnly);
            }
            assert!(recs.iter().all(|r| r.risk != RiskBucket::Safe));
        }
    }

    #[test]
    fn a_folder_with_protected_contents_is_never_grouped() {
        let dir = tempfile::tempdir().expect("tempdir");
        album(&dir.path().join("One"), b"bbbb");
        album(&dir.path().join("Two"), b"bbbb");
        let protected = ProtectedPaths::from_trees([dir.path().join("Two").join("sub")]);

        let found = scan_with(dir.path(), &protected, &HashSet::new());

        assert!(folders_of(&found)
            .iter()
            .all(|p| !p.starts_with(dir.path().join("Two"))));
    }

    #[test]
    fn the_ignore_list_survives_a_rescan() {
        let dir = tempfile::tempdir().expect("tempdir");
        album(&dir.path().join("One"), b"bbbb");
        album(&dir.path().join("Two"), b"bbbb");
        let conn = db::open_in_memory().expect("db");

        let first = scan(dir.path());
        let id = first.findings[0].meta["folderGroupId"].as_str().unwrap().to_owned();
        ignore_group(&conn, &id).expect("ignore");
        ignore_group(&conn, &id).expect("ignore twice is fine");

        let ignored = ignored_groups(&conn).expect("read");
        assert_eq!(ignored.len(), 1);
        let second = scan_with(dir.path(), &ProtectedPaths::empty(), &ignored);
        assert!(second.findings.is_empty());

        clear_ignored(&conn).expect("clear");
        let third = scan_with(
            dir.path(),
            &ProtectedPaths::empty(),
            &ignored_groups(&conn).expect("read"),
        );
        assert_eq!(folders_of(&third).len(), 2);
    }
}
