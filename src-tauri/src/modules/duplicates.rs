//! Duplicates: find files that really are the same bytes, suggest which copy to
//! keep, quarantine the rest (`docs/modules/03-duplicate-finder.md`).
//!
//! # Why the pipeline is staged
//!
//! Hashing a Pictures folder in full is minutes of disk I/O for an answer most
//! files could have been excluded from in microseconds. So the work narrows:
//!
//! 1. **Walk** the chosen roots. Skip directories, hidden and system files,
//!    anything the hard gates protect, our own quarantine, and anything below
//!    the size floor.
//! 2. **Group by exact size.** Two files of different sizes are never the same
//!    file, and this costs nothing — the walk already knows the sizes. Singleton
//!    groups are discarded here and never read at all.
//! 3. **Partial hash**: first 4 KB + last 4 KB, blake3. Two files that differ
//!    anywhere near either end separate here for 8 KB of reading each.
//! 4. **Full hash**: blake3 over the whole file, in parallel, with progress and
//!    cancellation. Only survivors of stage 3 get this far.
//! 5. **Identity**: two names for one file (a hard link, a junction) are not two
//!    files. Same volume serial + file index, so the second name is excluded and
//!    *reported* rather than quietly dropped.
//!
//! # Why a full hash and not just the partial one
//!
//! Because "same size, same first and last 4 KB" is a real shape, not a
//! hypothetical: disk images, padded archives, and files from the same template
//! all hit it. Stopping at stage 3 would offer to quarantine files that differ
//! in the middle. `a_crafted_collision_is_not_a_duplicate` builds exactly that
//! and asserts zero false positives.
//!
//! # Nothing here is ever Safe
//!
//! Module spec 03: "All removals = NeedsReview bucket minimum — duplicates are
//! never auto-Safe." A duplicate is a *judgement about a pair*, and the app has
//! no way to know which copy a person meant to keep. The keeper heuristic ranks;
//! the human decides.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::brain::{self, Brain, GateVerdict, ProtectedPaths};
// The same hash the executor recomputes to confirm a copy is still a copy
// (review H10) — one implementation, so the two numbers are comparable.
use crate::core::content_hash::full_hash_cancellable;
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::path_compare;
use crate::core::platform::{file_identity, EnvResolver, FileId};
use crate::core::scanner::{self, CancellationToken};
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket,
};

/// Files below this are not worth a person's attention (module spec 03).
pub const DEFAULT_MIN_SIZE_BYTES: u64 = 1024 * 1024;

/// The floor the setting itself is clamped to.
///
/// Zero would mean hashing every empty marker file on the disk, and every empty
/// file is a "duplicate" of every other. Validated in code so a settings page
/// cannot write a value that makes the scan useless.
pub const MIN_ALLOWED_SIZE_BYTES: u64 = 4 * 1024;

/// Bytes read from each end of a file for the stage-3 partial hash.
pub const PARTIAL_HASH_BYTES: u64 = 4 * 1024;

/// Folders inside the user profile that are scanned by default.
pub const DEFAULT_ROOT_NAMES: &[&str] = &["Downloads", "Documents", "Pictures", "Videos", "Desktop"];

/// Folder names that mean "passing through", so a copy sitting in one is a worse
/// keeper than the same bytes filed somewhere deliberate.
///
/// Compared as **component names**, not as paths: the question is "is one of
/// this path's folders called `Downloads`", which is about a name and not about
/// where two paths point, so it does not go through
/// [`crate::core::path_compare`] (standing rule, `docs/DECISIONS-NEEDED.md`).
const TRANSIENT_DIR_NAMES: &[&str] = &["downloads", "temp", "tmp", "cache", "desktop", "recent"];

/// Suffixes Windows and browsers add when they will not overwrite a file.
const COPY_MARKERS: &[&str] = &[" - copy", " copy", "-copy", "_copy", " (copy)"];

/* -------------------------------------------------------------------------
 * Settings
 * ---------------------------------------------------------------------- */

/// Per-module settings, stored under `module.duplicates`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DuplicatesSettings {
    /// Files smaller than this are not considered.
    pub min_size_bytes: u64,
    /// Roots to scan. Empty means [`default_roots`].
    pub roots: Vec<PathBuf>,
}

impl Default for DuplicatesSettings {
    fn default() -> Self {
        Self {
            min_size_bytes: DEFAULT_MIN_SIZE_BYTES,
            roots: Vec::new(),
        }
    }
}

impl DuplicatesSettings {
    /// The stored settings with every value forced into range.
    ///
    /// The same shape as `RetentionPolicy::validated`, and for the same reason:
    /// the settings table is a string store, and a `0` written into it must not
    /// change what the scanner considers.
    #[must_use]
    pub fn validated(self) -> Self {
        let min_size_bytes = self.min_size_bytes.max(MIN_ALLOWED_SIZE_BYTES);
        if min_size_bytes != self.min_size_bytes {
            tracing::warn!(
                stored = self.min_size_bytes,
                min_size_bytes,
                "duplicate minimum size was out of range and has been clamped"
            );
        }
        Self {
            min_size_bytes,
            ..self
        }
    }
}

/// The default scan roots on this machine: the folders module spec 03 names,
/// inside the user profile, that actually exist.
pub fn default_roots(env: &dyn EnvResolver) -> Vec<PathBuf> {
    let Some(profile) = env.var("USERPROFILE") else {
        return Vec::new();
    };
    let profile = PathBuf::from(profile);
    DEFAULT_ROOT_NAMES
        .iter()
        .map(|name| profile.join(name))
        .filter(|path| path.is_dir())
        .collect()
}

/* -------------------------------------------------------------------------
 * Results
 * ---------------------------------------------------------------------- */

/// One copy inside a [`DuplicateSet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateCopy {
    /// The finding this copy produced.
    pub finding_id: Uuid,
    /// Where it is.
    pub path: PathBuf,
    /// Creation time, when the filesystem exposes one.
    pub created: Option<DateTime<Utc>>,
    /// Last modification time.
    pub modified: Option<DateTime<Utc>>,
    /// Whether the heuristic suggests keeping this one.
    pub keeper: bool,
    /// The heuristic's score, so the UI can explain the ranking.
    pub keeper_score: i32,
    /// Why this copy scored as it did, in plain language.
    pub keeper_reason: String,
}

/// A set of files with identical content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateSet {
    /// Stable id for the UI.
    pub id: Uuid,
    /// Hex blake3 of the content, shared by every copy.
    pub content_hash: String,
    /// Size of one copy.
    pub size_bytes: u64,
    /// Every copy, keeper first.
    pub copies: Vec<DuplicateCopy>,
    /// What the redundant copies cost: `size * (copies - 1)`.
    pub wasted_bytes: u64,
}

/// A name that was excluded because it is another name for a file already in the
/// set (module spec 03, step 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedName {
    /// The name that was dropped.
    pub path: PathBuf,
    /// The name that was kept — the same file.
    pub same_file_as: PathBuf,
    /// Plain-language note, shown in the UI.
    pub note: String,
}

/// What one duplicate scan produced.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateScan {
    /// One finding per copy, including the keeper.
    pub findings: Vec<Finding>,
    /// Sets, largest waste first.
    pub sets: Vec<DuplicateSet>,
    /// Hard links and junctions excluded as second names for one file.
    pub linked_names: Vec<LinkedName>,
    /// Roots that could not be scanned, with the reason.
    pub skipped_roots: Vec<String>,
    /// Files whose contents were read for a full hash.
    pub files_hashed: u64,
    /// Candidates that could not be read for hashing, and so were left out. A
    /// cancelled hash is not counted: the user stopped it, the file did not fail.
    #[serde(default)]
    pub unreadable: u64,
    /// Whether the run stopped early.
    pub cancelled: bool,
}

impl DuplicateScan {
    /// Total bytes the redundant copies occupy.
    pub fn wasted_bytes(&self) -> u64 {
        self.sets.iter().map(|set| set.wasted_bytes).sum()
    }

    /// The same run, reported as stopped: a Stop that landed after the last
    /// cancel check still drops the answer, like any other cancelled scan.
    #[must_use]
    pub fn stopped(self) -> Self {
        Self {
            findings: Vec::new(),
            sets: Vec::new(),
            cancelled: true,
            ..self
        }
    }
}

/// Which stage a progress report came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    /// Walking the roots.
    Walking,
    /// Reading the first and last 4 KB of same-size candidates.
    PartialHashing,
    /// Reading survivors in full.
    FullHashing,
}

/// One progress tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateProgress {
    /// Which stage.
    pub stage: Stage,
    /// Items finished in this stage.
    pub done: u64,
    /// Items this stage has to get through. `0` while walking, which has no
    /// total until it finishes.
    pub total: u64,
    /// The path being looked at.
    pub current_path: PathBuf,
}

/// Progress callback.
///
/// `Fn` rather than `FnMut`, and `Sync`, because the full-hash stage calls it
/// from several rayon threads at once. Throttling is the caller's job.
pub type ProgressSink<'a> = &'a (dyn Fn(&DuplicateProgress) + Sync);

/// Everything a scan needs.
pub struct DuplicateContext<'a> {
    /// Roots to walk.
    pub roots: &'a [PathBuf],
    /// Hard gates. A root that resolves into a protected location is not walked.
    pub protected: &'a ProtectedPaths,
    /// Settings (size floor).
    pub settings: &'a DuplicatesSettings,
    /// Cancellation, checked between entries and between hashes.
    pub cancel: &'a CancellationToken,
}

/* -------------------------------------------------------------------------
 * The pipeline
 * ---------------------------------------------------------------------- */

/// One file that survived the walk.
#[derive(Debug, Clone)]
struct Candidate {
    path: PathBuf,
    size_bytes: u64,
    created: Option<DateTime<Utc>>,
    modified: Option<DateTime<Utc>>,
    accessed: Option<DateTime<Utc>>,
    identity: Option<FileId>,
}

/// Run the whole pipeline. Read-only: nothing here opens a file for writing.
pub fn scan(ctx: &DuplicateContext<'_>, progress: Option<ProgressSink<'_>>) -> DuplicateScan {
    let mut output = DuplicateScan::default();
    let min_size = ctx.settings.clone().validated().min_size_bytes;

    // Stage 1: walk.
    let mut candidates = Vec::new();
    let mut seen_paths = HashSet::new();
    for root in ctx.roots {
        if let Err(why) = resolve_root(root, ctx.protected) {
            tracing::warn!(root = %root.display(), why, "duplicate scan root refused");
            output.skipped_roots.push(why);
            continue;
        }

        // Name the root before the first candidate turns up: the UI learns
        // the scan id from progress, and a tree of small files has none.
        if let Some(report) = progress {
            report(&DuplicateProgress {
                stage: Stage::Walking,
                done: candidates.len() as u64,
                total: 0,
                current_path: root.clone(),
            });
        }

        let summary = scanner::walk(root, ctx.cancel, &mut |entry| {
            if entry.is_dir || entry.size_bytes < min_size {
                return;
            }
            if ctx.protected.reason_for(&entry.path).is_some() {
                return;
            }
            if is_hidden_or_system(&entry.path) {
                return;
            }
            // Two roots can overlap (`Documents` and a folder inside it), and one
            // file offered twice would look like a duplicate of itself. Through
            // `path_compare`, so two spellings of one file fold together.
            if !seen_paths.insert(path_compare::CanonicalPath::resolved(&entry.path).key()) {
                return;
            }
            if let Some(sink) = progress {
                sink(&DuplicateProgress {
                    stage: Stage::Walking,
                    done: candidates.len() as u64 + 1,
                    total: 0,
                    current_path: entry.path.clone(),
                });
            }
            candidates.push(Candidate {
                path: entry.path.clone(),
                size_bytes: entry.size_bytes,
                created: entry.created,
                modified: entry.modified,
                accessed: entry.accessed,
                identity: file_identity(&entry.path),
            });
        });
        output.cancelled |= summary.cancelled;
        if output.cancelled {
            return output;
        }
    }

    // Stage 2: group by exact size, discard singletons. Nothing has been read.
    let by_size = group_by(candidates, |candidate| candidate.size_bytes);

    // Stage 3: partial hash, first and last 4 KB.
    let stage3_total: u64 = by_size.iter().map(|group| group.len() as u64).sum();
    let mut stage3_done = 0u64;
    let mut survivors: Vec<Vec<Candidate>> = Vec::new();
    for group in by_size {
        if ctx.cancel.is_cancelled() {
            output.cancelled = true;
            return output;
        }
        let mut keyed = Vec::with_capacity(group.len());
        for candidate in group {
            stage3_done += 1;
            if let Some(sink) = progress {
                sink(&DuplicateProgress {
                    stage: Stage::PartialHashing,
                    done: stage3_done,
                    total: stage3_total,
                    current_path: candidate.path.clone(),
                });
            }
            match partial_hash(&candidate.path, candidate.size_bytes) {
                Ok(hash) => keyed.push((hash, candidate)),
                // Unreadable now is unreadable at execute time too; leaving it
                // out is the honest answer, not a silent loss of a duplicate.
                Err(err) => {
                    output.unreadable += 1;
                    tracing::debug!(
                        path = %candidate.path.display(),
                        error = %err,
                        "candidate skipped: could not read its ends"
                    );
                }
            }
        }
        survivors.extend(group_by(keyed, |(hash, _)| hash.clone()).into_iter().map(
            |group| group.into_iter().map(|(_, candidate)| candidate).collect(),
        ));
    }

    // Stage 4: full hash, in parallel, cancellable.
    let stage4_total: u64 = survivors.iter().map(|group| group.len() as u64).sum();
    let done = AtomicU64::new(0);
    let unreadable = AtomicU64::new(0);
    let flat: Vec<Candidate> = survivors.into_iter().flatten().collect();

    let hashed: Vec<Option<(String, Candidate)>> = flat
        .into_par_iter()
        .map(|candidate| {
            // Checked per file and, inside the hash, per chunk, so one huge file
            // cannot keep a cancelled scan alive.
            if ctx.cancel.is_cancelled() {
                return None;
            }
            let hash = match full_hash_cancellable(&candidate.path, &|| ctx.cancel.is_cancelled())
            {
                Ok(hash) => hash,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => return None,
                Err(err) => {
                    unreadable.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(
                        path = %candidate.path.display(),
                        error = %err,
                        "candidate skipped: could not be read in full"
                    );
                    return None;
                }
            };
            let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(sink) = progress {
                sink(&DuplicateProgress {
                    stage: Stage::FullHashing,
                    done: finished,
                    total: stage4_total,
                    current_path: candidate.path.clone(),
                });
            }
            Some((hash, candidate))
        })
        .collect();

    output.files_hashed = done.load(Ordering::Relaxed);
    output.unreadable += unreadable.load(Ordering::Relaxed);
    if ctx.cancel.is_cancelled() {
        output.cancelled = true;
        return output;
    }

    // Stage 5: identical content, grouped — then second names collapsed.
    for group in group_by(hashed.into_iter().flatten().collect(), |(hash, _)| {
        hash.clone()
    }) {
        let content_hash = group[0].0.clone();
        let members: Vec<Candidate> = group.into_iter().map(|(_, candidate)| candidate).collect();
        let (distinct, linked) = collapse_linked_names(members);
        output.linked_names.extend(linked);
        if distinct.len() < 2 {
            continue;
        }
        let (set, findings) = build_set(content_hash, distinct);
        output.findings.extend(findings);
        output.sets.push(set);
    }

    // Biggest waste first (module spec 03's UI ordering), with a deterministic
    // tie-break so two runs of the same disk produce the same list.
    output.sets.sort_by(|a, b| {
        b.wasted_bytes
            .cmp(&a.wasted_bytes)
            .then_with(|| a.content_hash.cmp(&b.content_hash))
    });
    output
}

/// Whether a scan root may be walked at all.
fn resolve_root(root: &Path, protected: &ProtectedPaths) -> Result<(), String> {
    if let Some(reason) = protected.reason_for(root) {
        return Err(format!(
            "`{}` was not scanned because it is a protected location ({reason})",
            root.display()
        ));
    }
    match brain::gate_path(root) {
        // A junction or an 8.3 alias can aim a harmless-looking root at a
        // protected tree; only the resolved form answers that.
        GateVerdict::Local(canonical) => match protected.reason_for(&canonical) {
            Some(reason) => Err(format!(
                "`{}` was not scanned because it resolves to `{}`, a protected location ({reason})",
                root.display(),
                canonical.display()
            )),
            None => Ok(()),
        },
        // A folder this machine does not have contributes nothing; not an error.
        GateVerdict::Missing => Ok(()),
        GateVerdict::Refused(why) => Err(format!(
            "`{}` was not scanned because {why}",
            root.display()
        )),
    }
}

/// Group items by a key, discarding groups of one. Deterministic order.
fn group_by<T, K: Ord>(items: Vec<T>, key: impl Fn(&T) -> K) -> Vec<Vec<T>> {
    let mut buckets: BTreeMap<K, Vec<T>> = BTreeMap::new();
    for item in items {
        buckets.entry(key(&item)).or_default().push(item);
    }
    buckets
        .into_values()
        .filter(|group| group.len() > 1)
        .collect()
}

/// Drop every name that is a second name for a file already in the group.
///
/// A hard link and a junction give one file two names. Quarantining "the copy"
/// would move the only copy, so the second name is excluded — and reported,
/// because a user who made that link deliberately deserves to be told why their
/// obvious duplicate is not listed.
fn collapse_linked_names(members: Vec<Candidate>) -> (Vec<Candidate>, Vec<LinkedName>) {
    let mut kept: Vec<Candidate> = Vec::with_capacity(members.len());
    let mut linked = Vec::new();
    let mut identities: HashMap<FileId, PathBuf> = HashMap::new();

    for candidate in members {
        // No identity means the filesystem would not say. Keeping the candidate
        // is the safe direction: it stays in the set, the user still decides,
        // and nothing is silently assumed to be unique.
        let Some(identity) = candidate.identity else {
            kept.push(candidate);
            continue;
        };
        match identities.get(&identity) {
            Some(first) => linked.push(LinkedName {
                note: format!(
                    "`{}` is the same file as `{}` under another name (a hard link or junction), \
                     not a second copy, so removing it would not free anything.",
                    candidate.path.display(),
                    first.display()
                ),
                same_file_as: first.clone(),
                path: candidate.path,
            }),
            None => {
                identities.insert(identity, candidate.path.clone());
                kept.push(candidate);
            }
        }
    }
    (kept, linked)
}

/// Turn a group of identical files into a set plus its findings.
fn build_set(content_hash: String, members: Vec<Candidate>) -> (DuplicateSet, Vec<Finding>) {
    let set_id = Uuid::new_v4();
    let size_bytes = members.first().map_or(0, |c| c.size_bytes);
    let oldest = members.iter().filter_map(|c| c.created).min();

    let mut scored: Vec<(i32, String, Candidate)> = members
        .into_iter()
        .map(|candidate| {
            let (score, reason) = keeper_score(&candidate, oldest);
            (score, reason, candidate)
        })
        .collect();

    // Highest score wins; ties break on the shortest path and then on the path
    // itself, so the same disk always nominates the same keeper.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.2.path.as_os_str().len().cmp(&b.2.path.as_os_str().len()))
            .then_with(|| a.2.path.cmp(&b.2.path))
    });

    // Counted before the loop consumes `scored`. This used to report
    // `copies.capacity()`, which is the allocator's business and not the set's:
    // it happened to equal the real count only because `Vec::with_capacity` had
    // been asked for exactly that many and nothing had grown it (review M15).
    // Every finding in a set must agree about how big the set is.
    let total = scored.len();
    let mut copies = Vec::with_capacity(total);
    let mut findings = Vec::with_capacity(total);

    // The keeper is whichever copy sorted first, and every other finding records
    // it by name. The executor needs it to answer a question the analyze phase
    // cannot: is the copy this one was going to be measured against *still
    // there* when the user finally clicks clean? Quarantining the last remaining
    // copy under the reason "identical to <keeper>" is the failure H10 is about.
    let keeper_path = scored.first().map(|(_, _, candidate)| candidate.path.clone());

    for (index, (score, reason, candidate)) in scored.into_iter().enumerate() {
        let keeper = index == 0;
        let mut finding = Finding::new("duplicates", candidate.path.clone(), FindingKind::File);
        finding.size_bytes = candidate.size_bytes;
        finding.created = candidate.created;
        finding.modified = candidate.modified;
        finding.accessed = candidate.accessed;
        finding.meta = serde_json::json!({
            "setId": set_id,
            "contentHash": content_hash,
            "keeper": keeper,
            "copiesInSet": total,
            "keeperPath": keeper_path,
        });

        copies.push(DuplicateCopy {
            finding_id: finding.id,
            path: candidate.path,
            created: candidate.created,
            modified: candidate.modified,
            keeper,
            keeper_score: score,
            keeper_reason: reason,
        });
        findings.push(finding);
    }

    let redundant = copies.len().saturating_sub(1) as u64;
    (
        DuplicateSet {
            id: set_id,
            content_hash,
            size_bytes,
            wasted_bytes: size_bytes.saturating_mul(redundant),
            copies,
        },
        findings,
    )
}

/// Score one copy as a keeper, with the sentence that explains the score.
///
/// The four signals module spec 03 asks for: an older `created` (the original),
/// depth in an organised folder rather than a passing-through one, a name with
/// no ` - Copy` marker, and a folder with siblings around it.
fn keeper_score(candidate: &Candidate, oldest: Option<DateTime<Utc>>) -> (i32, String) {
    let mut score = 0;
    let mut notes: Vec<&str> = Vec::new();

    let name = candidate
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);

    if !has_copy_marker(stem) {
        score += 30;
        notes.push("its name has no \"copy\" marker");
    }

    let transient = candidate.path.components().any(|part| {
        let text = part.as_os_str().to_string_lossy().to_lowercase();
        TRANSIENT_DIR_NAMES.contains(&text.as_str())
    });
    if transient {
        notes.push("it sits in a folder things pass through");
    } else {
        score += 20;
        notes.push("it is filed somewhere deliberate");
    }

    // Depth in an organised tree: `Pictures\2023\trip\a.jpg` beats
    // `Downloads\a.jpg`. Capped, so burying a file does not win by itself.
    let depth = candidate.path.components().count().saturating_sub(2);
    score += 5 * i32::try_from(depth.min(4)).unwrap_or(0);

    if let (Some(created), Some(oldest)) = (candidate.created, oldest) {
        if created == oldest {
            score += 15;
            notes.push("it is the oldest copy");
        }
    }

    // A folder with other files in it looks like a collection; one holding
    // nothing else looks like a place something was dropped.
    let siblings = candidate
        .path
        .parent()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map_or(0, |entries| entries.take(64).count());
    score += i32::try_from((siblings / 8).min(5)).unwrap_or(0);

    (score, format!("Suggested keeper because {}.", notes.join(", ")))
}

fn has_copy_marker(stem: &str) -> bool {
    if COPY_MARKERS.iter().any(|marker| stem.ends_with(marker)) {
        return true;
    }
    // ` (1)`, ` (2)` … the shape Windows and every browser add.
    match stem.rsplit_once(" (") {
        Some((_, tail)) => {
            tail.ends_with(')') && tail[..tail.len() - 1].chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// blake3 of the first and last [`PARTIAL_HASH_BYTES`] of a file.
///
/// The size is mixed in too, so two files of different sizes can never collide
/// here even if both are shorter than the window.
fn partial_hash(path: &Path, size: u64) -> std::io::Result<String> {
    use std::io::{Seek, SeekFrom};

    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());

    let window = PARTIAL_HASH_BYTES.min(size) as usize;
    let mut head = vec![0u8; window];
    file.read_exact(&mut head)?;
    hasher.update(&head);

    if size > PARTIAL_HASH_BYTES {
        let mut tail = vec![0u8; window];
        file.seek(SeekFrom::End(-(window as i64)))?;
        file.read_exact(&mut tail)?;
        hasher.update(&tail);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Whether a file carries the hidden or system attribute.
///
/// Module spec 03 skips both: they are the operating system's business, and a
/// user who hid a file did not hide it in order to be offered it back.
fn is_hidden_or_system(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

        std::fs::metadata(path).is_ok_and(|metadata| {
            metadata.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0
        })
    }

    #[cfg(not(windows))]
    {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
    }
}

/* -------------------------------------------------------------------------
 * analyze
 * ---------------------------------------------------------------------- */

/// Turn a duplicate scan into recommendations.
///
/// **Never Safe.** The keeper is information only; every other copy is offered
/// at `NeedsReview` and no higher, whatever the brain's gates would otherwise
/// have said. A gate can still push one *down* to red — a copy that turns out to
/// sit in a protected location is refused like anything else.
pub fn analyze(scan: &DuplicateScan, brain: &Brain) -> Vec<Recommendation> {
    let mut recommendations = Vec::with_capacity(scan.findings.len());

    for set in &scan.sets {
        for copy in &set.copies {
            let refusal = brain.path_refusal(&copy.path);
            let lock = brain.lock_state(&copy.path);

            let (risk, action, reason) = if let Some(refusal) = refusal {
                (RiskBucket::DoNotDelete, Action::InfoOnly, refusal.reason)
            } else if !lock.is_free() {
                (
                    RiskBucket::DoNotDelete,
                    Action::InfoOnly,
                    format!("left alone: {}", lock.reason()),
                )
            } else if copy.keeper {
                (
                    RiskBucket::NeedsReview,
                    Action::InfoOnly,
                    format!(
                        "{} The other {} in this set have identical contents.",
                        copy.keeper_reason,
                        plural(set.copies.len() - 1, "copy", "copies"),
                    ),
                )
            } else {
                (
                    RiskBucket::NeedsReview,
                    Action::Quarantine,
                    format!(
                        "Byte-for-byte identical to `{}`, which is the suggested keeper. \
                         Duplicates are never removed automatically — pick the copy you want gone.",
                        set.copies
                            .iter()
                            .find(|c| c.keeper)
                            .map_or_else(|| "the other copy".to_owned(), |c| c
                                .path
                                .display()
                                .to_string()),
                    ),
                )
            };

            let savings = if action.is_mutating() {
                set.size_bytes
            } else {
                0
            };

            // The machine-checkable half of the reason: the executor re-hashes
            // this copy and confirms the keeper is still on disk before it moves
            // anything (architect addition A, phase 3). It is the same shape the
            // finding's `meta` carries for the UI — one is for a person, one is
            // for gate 4.
            let justification = Justification::DuplicateOf {
                content_hash: set.content_hash.clone(),
                keeper: copy.keeper,
                keeper_path: set
                    .copies
                    .iter()
                    .find(|c| c.keeper)
                    .map(|c| c.path.clone()),
            };

            match Recommendation::new(
                vec![copy.finding_id],
                risk,
                DUPLICATE_CONFIDENCE,
                action,
                reason,
                savings,
                justification,
            ) {
                Ok(rec) => recommendations.push(rec),
                Err(err) => tracing::error!(
                    path = %copy.path.display(),
                    error = %err,
                    "dropped a duplicate recommendation that failed its own invariants"
                ),
            }
        }
    }

    recommendations
}

/// Confidence reported for a duplicate.
///
/// Not a score from `brain::score`: a blake3 match is a *fact* about the bytes,
/// not a heuristic, and the uncertainty is entirely about which copy a person
/// wants — which is why the bucket is `NeedsReview` and the number is not 100.
pub const DUPLICATE_CONFIDENCE: u8 = 75;

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
    }
}

/* -------------------------------------------------------------------------
 * Registry
 * ---------------------------------------------------------------------- */

/// Registry entry.
#[derive(Debug, Default)]
pub struct DuplicatesModule;

impl ToolModule for DuplicatesModule {
    fn id(&self) -> &'static str {
        "duplicates"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Duplicate Finder".to_owned(),
            description:
                "Find files with byte-for-byte identical contents, see which copy to keep, and quarantine the rest."
                    .to_owned(),
            icon: "copies".to_owned(),
            capabilities: ModuleCapabilities::FULL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::content_hash::full_hash;
    use crate::core::platform::{AlwaysUnlocked, MapEnv, StubProcessProbe};
    use std::fs;
    use std::sync::Arc;

    fn write(path: &Path, bytes: &[u8]) -> PathBuf {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, bytes).expect("write");
        path.to_path_buf()
    }

    /// A payload big enough to clear the size floor, filled from a seed so two
    /// files can be made to differ only in the middle.
    fn payload(size: usize, seed: u8, middle: Option<u8>) -> Vec<u8> {
        let mut bytes = vec![seed; size];
        if let Some(value) = middle {
            bytes[size / 2] = value;
        }
        bytes
    }

    fn settings(min_size: u64) -> DuplicatesSettings {
        DuplicatesSettings {
            min_size_bytes: min_size,
            roots: Vec::new(),
        }
    }

    fn run(root: &Path, min_size: u64) -> DuplicateScan {
        run_with(root, min_size, &CancellationToken::new(), None)
    }

    fn run_with(
        root: &Path,
        min_size: u64,
        cancel: &CancellationToken,
        progress: Option<ProgressSink<'_>>,
    ) -> DuplicateScan {
        let protected = ProtectedPaths::empty();
        let settings = settings(min_size);
        scan(
            &DuplicateContext {
                roots: std::slice::from_ref(&root.to_path_buf()),
                protected: &protected,
                settings: &settings,
                cancel,
            },
            progress,
        )
    }

    fn brain() -> Brain {
        Brain::builder()
            .lock_probe(Arc::new(AlwaysUnlocked))
            .process_probe(Arc::new(StubProcessProbe::none()))
            .build()
    }

    fn paths_in(set: &DuplicateSet) -> Vec<String> {
        let mut names: Vec<_> = set
            .copies
            .iter()
            .map(|c| {
                c.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn identical_files_are_one_set_and_different_ones_are_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let same = payload(64 * 1024, 7, None);
        write(&dir.path().join("Pictures/a.bin"), &same);
        write(&dir.path().join("Downloads/a copy.bin"), &same);
        write(&dir.path().join("Pictures/other.bin"), &payload(64 * 1024, 9, None));

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);

        assert_eq!(scan.sets.len(), 1, "one set: {:?}", scan.sets);
        assert_eq!(paths_in(&scan.sets[0]), vec!["a copy.bin", "a.bin"]);
        assert_eq!(scan.sets[0].size_bytes, 64 * 1024);
        assert_eq!(scan.sets[0].wasted_bytes, 64 * 1024);
        assert_eq!(scan.wasted_bytes(), 64 * 1024);
        assert!(!scan.cancelled);
    }

    /// **The acceptance criterion**: same size, same first and last 4 KB,
    /// different in the middle. Stage 3 cannot tell them apart, so stage 4 has
    /// to — and if it did not, the app would offer to quarantine a file that is
    /// not a copy of anything.
    ///
    /// Prints what each stage concluded, so the evidence is in the output.
    #[test]
    fn a_crafted_collision_is_not_a_duplicate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let size = 64 * 1024;

        // Three files, identical everywhere the partial hash looks.
        let a = write(&dir.path().join("a.bin"), &payload(size, 0xAB, Some(0x01)));
        let b = write(&dir.path().join("b.bin"), &payload(size, 0xAB, Some(0x02)));
        let c = write(&dir.path().join("c.bin"), &payload(size, 0xAB, Some(0x03)));
        // …and a genuine pair, so the test proves the stage still finds real ones.
        let real = payload(size, 0xCD, None);
        write(&dir.path().join("real-1.bin"), &real);
        write(&dir.path().join("real-2.bin"), &real);

        let partial_a = partial_hash(&a, size as u64).expect("hash");
        let partial_b = partial_hash(&b, size as u64).expect("hash");
        let partial_c = partial_hash(&c, size as u64).expect("hash");
        println!("stage 3 (first+last 4 KB): a={partial_a}");
        println!("stage 3 (first+last 4 KB): b={partial_b}");
        println!("stage 3 (first+last 4 KB): c={partial_c}");
        assert_eq!(
            partial_a, partial_b,
            "the premise: stage 3 cannot separate these"
        );
        assert_eq!(partial_a, partial_c);

        println!("stage 4 (full blake3):     a={}", full_hash(&a).expect("hash"));
        println!("stage 4 (full blake3):     b={}", full_hash(&b).expect("hash"));
        println!("stage 4 (full blake3):     c={}", full_hash(&c).expect("hash"));

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        for set in &scan.sets {
            println!("set: {:?} wasted={}", paths_in(set), set.wasted_bytes);
        }

        assert_eq!(
            scan.sets.len(),
            1,
            "only the genuine pair may be reported: {:?}",
            scan.sets.iter().map(paths_in).collect::<Vec<_>>()
        );
        assert_eq!(paths_in(&scan.sets[0]), vec!["real-1.bin", "real-2.bin"]);

        // Said as an invariant rather than only as a count.
        for set in &scan.sets {
            for copy in &set.copies {
                assert!(
                    !copy.path.ends_with("a.bin")
                        && !copy.path.ends_with("b.bin")
                        && !copy.path.ends_with("c.bin"),
                    "`{}` differs in the middle and is nobody's duplicate",
                    copy.path.display()
                );
            }
        }
    }

    /// **The acceptance criterion**: a hard link is one file with two names.
    /// Offering to remove "the copy" would remove the only copy.
    #[test]
    fn a_hardlinked_pair_is_excluded_and_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 3, None);
        let original = write(&dir.path().join("original.bin"), &bytes);
        let link = dir.path().join("second-name.bin");
        fs::hard_link(&original, &link).expect("hard links need no privilege on NTFS");

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);

        assert!(
            scan.sets.is_empty(),
            "one file under two names is not a duplicate set: {:?}",
            scan.sets.iter().map(paths_in).collect::<Vec<_>>()
        );
        assert_eq!(scan.linked_names.len(), 1, "and it must be reported");
        let note = &scan.linked_names[0];
        println!("excluded: {}", note.note);
        assert!(note.note.contains("hard link"), "got `{}`", note.note);
        assert!(note.note.contains("not a second copy"), "got `{}`", note.note);
        assert!(original.exists() && link.exists());
    }

    /// A hard link *inside* a real duplicate set: the third name collapses into
    /// the file it points at, and the genuine second copy still stands.
    #[test]
    fn a_link_inside_a_real_set_collapses_without_losing_the_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 5, None);
        let first = write(&dir.path().join("first.bin"), &bytes);
        write(&dir.path().join("second.bin"), &bytes);
        fs::hard_link(&first, dir.path().join("first-again.bin")).expect("hard link");

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);

        assert_eq!(scan.sets.len(), 1);
        let names = paths_in(&scan.sets[0]);
        assert_eq!(names.len(), 2, "two files, not three names: {names:?}");
        assert!(names.contains(&"second.bin".to_owned()), "{names:?}");
        // Either name of the linked file is a correct representative; which one
        // survives is decided by the walk order and is therefore stable.
        assert!(
            names.contains(&"first.bin".to_owned())
                || names.contains(&"first-again.bin".to_owned()),
            "{names:?}"
        );
        assert_eq!(scan.linked_names.len(), 1);
        assert_eq!(
            scan.sets[0].wasted_bytes,
            32 * 1024,
            "the link must not be counted as wasted space"
        );
    }

    /// **The acceptance criterion**: a 10k-file fixture completes, reports
    /// progress, and can be cancelled part-way through the hashing.
    #[test]
    fn a_ten_thousand_file_fixture_completes_with_progress() {
        let dir = tempfile::tempdir().expect("tempdir");
        const FILES: usize = 10_000;
        const SIZE: usize = 5 * 1024;

        // 5000 distinct payloads, each written twice: 5000 real duplicate sets,
        // and every file has to be hashed to prove it.
        for index in 0..FILES / 2 {
            let bytes = payload(SIZE, (index % 251) as u8, Some((index % 253) as u8));
            let dir_index = index % 32;
            write(
                &dir.path().join(format!("d{dir_index}/f{index}-1.bin")),
                &bytes,
            );
            write(
                &dir.path().join(format!("d{dir_index}/f{index}-2.bin")),
                &bytes,
            );
        }

        let stages = std::sync::Mutex::new(Vec::<(Stage, u64, u64)>::new());
        let sink = |progress: &DuplicateProgress| {
            let mut stages = stages.lock().unwrap_or_else(|e| e.into_inner());
            stages.push((progress.stage, progress.done, progress.total));
        };

        let started = std::time::Instant::now();
        let scan = run_with(
            dir.path(),
            MIN_ALLOWED_SIZE_BYTES,
            &CancellationToken::new(),
            Some(&sink),
        );
        let elapsed = started.elapsed();

        let ticks = stages.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let count = |stage: Stage| ticks.iter().filter(|(s, _, _)| *s == stage).count();
        println!(
            "10k fixture: {} sets, {} files hashed, {:?}; ticks walking={} partial={} full={}",
            scan.sets.len(),
            scan.files_hashed,
            elapsed,
            count(Stage::Walking),
            count(Stage::PartialHashing),
            count(Stage::FullHashing),
        );

        assert!(!scan.cancelled);
        assert_eq!(scan.sets.len(), FILES / 2, "every pair must be found");
        assert_eq!(scan.findings.len(), FILES);
        assert_eq!(scan.files_hashed, FILES as u64);

        // Progress must actually be reported, in every stage, and the full-hash
        // stage must know its total.
        // One walking tick per file, plus the one that names the root up front.
        assert_eq!(count(Stage::Walking), FILES + 1);
        assert_eq!(count(Stage::PartialHashing), FILES);
        assert_eq!(count(Stage::FullHashing), FILES);
        assert!(ticks
            .iter()
            .filter(|(stage, _, _)| *stage == Stage::FullHashing)
            .all(|(_, done, total)| *total == FILES as u64 && *done <= *total));
    }

    /// Cancellation **mid-hash**: the token is tripped from another thread once
    /// the full-hash stage has started, and the scan stops rather than finishing
    /// the remaining files.
    #[test]
    fn cancellation_during_hashing_stops_the_scan() {
        let dir = tempfile::tempdir().expect("tempdir");
        const PAIRS: usize = 800;
        // Big enough that hashing takes long enough to be interrupted, small
        // enough that the fixture is ~100 MB rather than most of a gigabyte.
        const SIZE: usize = 64 * 1024;

        for index in 0..PAIRS {
            let bytes = payload(SIZE, (index % 251) as u8, Some((index % 253) as u8));
            write(&dir.path().join(format!("d{}/f{index}-1.bin", index % 16)), &bytes);
            write(&dir.path().join(format!("d{}/f{index}-2.bin", index % 16)), &bytes);
        }

        let cancel = CancellationToken::new();
        let reached_full_hashing = std::sync::atomic::AtomicBool::new(false);
        let cancelled_at = AtomicU64::new(0);

        let sink = |progress: &DuplicateProgress| {
            if progress.stage == Stage::FullHashing {
                reached_full_hashing.store(true, Ordering::SeqCst);
                // Trip it a few files in, so the stop is genuinely mid-stage.
                if progress.done >= 8 && !cancel.is_cancelled() {
                    cancelled_at.store(progress.done, Ordering::SeqCst);
                    cancel.cancel();
                }
            }
        };

        let scan = run_with(dir.path(), MIN_ALLOWED_SIZE_BYTES, &cancel, Some(&sink));

        println!(
            "cancelled after {} of {} files; hashed {}, sets {}",
            cancelled_at.load(Ordering::SeqCst),
            PAIRS * 2,
            scan.files_hashed,
            scan.sets.len(),
        );

        assert!(
            reached_full_hashing.load(Ordering::SeqCst),
            "the fixture must reach the full-hash stage for this to mean anything"
        );
        assert!(scan.cancelled, "a cancelled scan must say so");
        assert!(
            scan.sets.is_empty(),
            "a cancelled scan must not offer half an answer"
        );
        assert!(
            scan.files_hashed < (PAIRS * 2) as u64,
            "cancellation must stop work: hashed {} of {}",
            scan.files_hashed,
            PAIRS * 2
        );
    }

    /// Audit A8: a candidate that cannot be opened is counted, not silently lost.
    #[cfg(windows)]
    #[test]
    fn an_unreadable_candidate_is_counted_in_the_scan() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 21, None);
        write(&dir.path().join("a.bin"), &bytes);
        write(&dir.path().join("b.bin"), &bytes);
        let locked = dir.path().join("c.bin");
        write(&locked, &bytes);
        // Exclusive handle: every other open fails with a sharing violation.
        let _hold = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .expect("hold");

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        assert_eq!(scan.unreadable, 1, "{scan:?}");
        assert_eq!(scan.sets.len(), 1, "the readable pair is still found");
    }

    /// The UI only learns the scan id from a progress event, so a Stop pressed
    /// before the first one waits for it. A tree with no qualifying file must
    /// still produce that first event, or the Stop is never sent.
    #[test]
    fn a_scan_reports_its_root_before_finding_any_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("tiny.txt"), b"too small to qualify");
        let events = std::sync::Mutex::new(Vec::new());
        let sink = |progress: &DuplicateProgress| {
            events.lock().expect("events").push(progress.clone());
        };

        run_with(
            dir.path(),
            MIN_ALLOWED_SIZE_BYTES,
            &CancellationToken::new(),
            Some(&sink),
        );

        let events = events.into_inner().expect("events");
        let first = events.first().expect("at least one progress event");
        assert_eq!(first.stage, Stage::Walking);
        assert_eq!(first.done, 0);
        assert_eq!(first.current_path, dir.path());
    }

    #[test]
    fn a_stopped_scan_keeps_no_half_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 23, None);
        write(&dir.path().join("a.bin"), &bytes);
        write(&dir.path().join("b.bin"), &bytes);
        let full = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        assert_eq!(full.sets.len(), 1);

        let stopped = full.stopped();

        assert!(stopped.cancelled);
        assert!(stopped.sets.is_empty());
        assert!(stopped.findings.is_empty());
        assert_eq!(stopped.wasted_bytes(), 0);
    }

    #[test]
    fn a_cancelled_scan_does_not_count_unreadable_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 22, None);
        write(&dir.path().join("a.bin"), &bytes);
        write(&dir.path().join("b.bin"), &bytes);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let scan = run_with(dir.path(), MIN_ALLOWED_SIZE_BYTES, &cancel, None);
        assert!(scan.cancelled);
        assert_eq!(scan.unreadable, 0);
    }

    #[test]
    fn cancellation_before_hashing_stops_the_walk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 1, None);
        write(&dir.path().join("a.bin"), &bytes);
        write(&dir.path().join("b.bin"), &bytes);

        let cancel = CancellationToken::new();
        cancel.cancel();
        let scan = run_with(dir.path(), MIN_ALLOWED_SIZE_BYTES, &cancel, None);

        assert!(scan.cancelled);
        assert!(scan.sets.is_empty());
        assert_eq!(scan.files_hashed, 0);
    }

    #[test]
    fn files_below_the_size_floor_are_never_considered() {
        let dir = tempfile::tempdir().expect("tempdir");
        let small = payload(1_024, 2, None);
        write(&dir.path().join("small-a.bin"), &small);
        write(&dir.path().join("small-b.bin"), &small);
        let big = payload(64 * 1024, 2, None);
        write(&dir.path().join("big-a.bin"), &big);
        write(&dir.path().join("big-b.bin"), &big);

        let scan = run(dir.path(), 32 * 1024);
        assert_eq!(scan.sets.len(), 1);
        assert_eq!(paths_in(&scan.sets[0]), vec!["big-a.bin", "big-b.bin"]);
        assert_eq!(scan.files_hashed, 2, "the small pair is never read");
    }

    /// A settings page must not be able to write a floor of zero, for the same
    /// reason it must not be able to write a retention of zero: every empty file
    /// is a duplicate of every other, and the scan becomes noise.
    #[test]
    fn the_size_floor_is_validated_not_trusted() {
        assert_eq!(
            settings(0).validated().min_size_bytes,
            MIN_ALLOWED_SIZE_BYTES
        );
        assert_eq!(
            settings(1).validated().min_size_bytes,
            MIN_ALLOWED_SIZE_BYTES
        );
        assert_eq!(
            settings(DEFAULT_MIN_SIZE_BYTES).validated().min_size_bytes,
            DEFAULT_MIN_SIZE_BYTES
        );
        assert_eq!(
            DuplicatesSettings::default().min_size_bytes,
            DEFAULT_MIN_SIZE_BYTES
        );

        // And the scan uses the validated value, not the stored one.
        let dir = tempfile::tempdir().expect("tempdir");
        let tiny = payload(64, 1, None);
        write(&dir.path().join("t-a.bin"), &tiny);
        write(&dir.path().join("t-b.bin"), &tiny);
        assert!(run(dir.path(), 0).sets.is_empty());
    }

    #[test]
    fn hidden_and_system_files_are_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 4, None);
        let visible = write(&dir.path().join("visible.bin"), &bytes);
        let hidden = write(&dir.path().join("hidden.bin"), &bytes);

        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd.exe")
                .arg("/C")
                .arg("attrib")
                .arg("+H")
                .arg(hidden.to_string_lossy().replace('/', "\\"))
                .output()
                .expect("attrib");
            assert!(output.status.success(), "could not hide the fixture file");
        }
        #[cfg(not(windows))]
        let hidden = {
            let dotted = dir.path().join(".hidden.bin");
            fs::rename(&hidden, &dotted).expect("rename");
            dotted
        };

        assert!(is_hidden_or_system(&hidden), "the premise");
        assert!(!is_hidden_or_system(&visible));

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        assert!(
            scan.sets.is_empty(),
            "a hidden file must not be offered: {:?}",
            scan.sets.iter().map(paths_in).collect::<Vec<_>>()
        );
    }

    /// Module spec 03's keeper heuristic, as a table.
    #[test]
    fn the_keeper_heuristic_prefers_the_filed_original() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 6, None);

        let filed = write(&dir.path().join("Pictures/2023/trip/photo.jpg"), &bytes);
        write(&dir.path().join("Downloads/photo (1).jpg"), &bytes);
        write(&dir.path().join("Downloads/photo - Copy.jpg"), &bytes);

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        assert_eq!(scan.sets.len(), 1);
        let set = &scan.sets[0];

        println!("keeper ranking:");
        for copy in &set.copies {
            println!(
                "  {:>4}  {}  {}",
                copy.keeper_score,
                if copy.keeper { "KEEP" } else { "    " },
                copy.path.display()
            );
        }

        assert!(set.copies[0].keeper, "the first copy is the keeper");
        assert_eq!(
            set.copies[0].path, filed,
            "the filed original must win over two Downloads copies"
        );
        assert_eq!(
            set.copies.iter().filter(|c| c.keeper).count(),
            1,
            "exactly one keeper per set"
        );
        assert!(!set.copies[0].keeper_reason.trim().is_empty());
    }

    #[test]
    fn copy_markers_are_recognised() {
        for stem in [
            "photo - copy",
            "photo copy",
            "photo-copy",
            "photo_copy",
            "photo (1)",
            "photo (12)",
        ] {
            assert!(has_copy_marker(stem), "`{stem}` is a copy marker");
        }
        for stem in [
            "photo",
            "copy of the report",
            "photo (draft)",
            "photo (1a)",
            "2023 (holiday)",
        ] {
            assert!(!has_copy_marker(stem), "`{stem}` is not a copy marker");
        }
    }

    /// The rule that has no exceptions: **duplicates are never Safe.**
    #[test]
    fn no_duplicate_recommendation_is_ever_safe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(64 * 1024, 8, None);
        write(&dir.path().join("Pictures/keep.bin"), &bytes);
        write(&dir.path().join("Downloads/copy-a.bin"), &bytes);
        write(&dir.path().join("Downloads/copy-b.bin"), &bytes);

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        let recommendations = analyze(&scan, &brain());

        assert_eq!(recommendations.len(), 3);
        for rec in &recommendations {
            assert_ne!(rec.risk, RiskBucket::Safe, "a duplicate went green");
            assert!(!rec.reason.trim().is_empty());
        }

        let quarantined: Vec<_> = recommendations
            .iter()
            .filter(|rec| rec.action == Action::Quarantine)
            .collect();
        assert_eq!(quarantined.len(), 2, "the keeper is not offered for removal");
        for rec in quarantined {
            assert_eq!(rec.risk, RiskBucket::NeedsReview);
            assert_eq!(rec.savings_bytes, 64 * 1024);
            assert!(rec.reason.contains("never removed automatically"));
        }
    }

    /// The gates still apply on top: a copy sitting in a protected location is
    /// red and offers nothing, even though its content really is duplicated.
    #[test]
    fn a_copy_in_a_protected_location_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(64 * 1024, 11, None);
        write(&dir.path().join("Pictures/keep.bin"), &bytes);
        let planted = write(&dir.path().join("Downloads/also.bin"), &bytes);

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        assert_eq!(scan.sets.len(), 1);

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([dir.path().join("Downloads")]))
            .lock_probe(Arc::new(AlwaysUnlocked))
            .build();
        let recommendations = analyze(&scan, &brain);

        let for_planted = scan.sets[0]
            .copies
            .iter()
            .find(|c| c.path == planted)
            .expect("the planted copy");
        let rec = recommendations
            .iter()
            .find(|rec| rec.finding_ids.contains(&for_planted.finding_id))
            .expect("its recommendation");

        assert_eq!(rec.risk, RiskBucket::DoNotDelete);
        assert_eq!(rec.action, Action::InfoOnly);
        assert_eq!(rec.savings_bytes, 0);
        assert!(rec.reason.contains("protected"));
    }

    /// A protected root is never walked, and the scan says so rather than
    /// quietly finding nothing.
    #[test]
    fn a_protected_root_is_refused_and_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 12, None);
        write(&dir.path().join("a.bin"), &bytes);
        write(&dir.path().join("b.bin"), &bytes);

        let protected = ProtectedPaths::from_trees([dir.path().to_path_buf()]);
        let settings = settings(MIN_ALLOWED_SIZE_BYTES);
        let cancel = CancellationToken::new();
        let scan = scan(
            &DuplicateContext {
                roots: std::slice::from_ref(&dir.path().to_path_buf()),
                protected: &protected,
                settings: &settings,
                cancel: &cancel,
            },
            None,
        );

        assert!(scan.sets.is_empty());
        assert_eq!(scan.skipped_roots.len(), 1);
        assert!(scan.skipped_roots[0].contains("protected location"));
    }

    /// Every spelling the gate cannot judge is refused, on any machine and with
    /// no reparse point needed.
    #[test]
    fn a_root_the_gate_cannot_judge_is_refused() {
        let protected = ProtectedPaths::empty();
        for root in [
            "\\\\localhost\\C$\\Users\\dev\\Pictures",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Pictures",
            "\\??\\C:\\Pictures",
            "C:Pictures",
            "\\Pictures",
        ] {
            assert!(
                resolve_root(Path::new(root), &protected).is_err(),
                "`{root}` must be refused"
            );
        }
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(resolve_root(dir.path(), &protected).is_ok());
    }

    /// The same guarantee through a junction, which needs no privilege.
    #[cfg(windows)]
    #[test]
    fn a_root_that_is_a_junction_into_a_protected_location_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("Windows");
        fs::create_dir_all(&real).expect("dir");
        let door = dir.path().join("innocent");
        assert!(
            crate::core::test_support::make_junction(&door, &real),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );

        let protected = ProtectedPaths::from_trees([real]);
        assert!(
            protected.reason_for(&door).is_none(),
            "the premise: the junction's own path is not lexically protected"
        );
        let err = resolve_root(&door, &protected).expect_err("a junction must be resolved first");
        assert!(err.contains("resolves to"), "got `{err}`");
    }

    #[test]
    fn scanning_never_modifies_anything() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 13, None);
        write(&dir.path().join("a.bin"), &bytes);
        write(&dir.path().join("sub/b.bin"), &bytes);

        let snapshot = |root: &Path| {
            let cancel = CancellationToken::new();
            let mut entries = Vec::new();
            scanner::walk(root, &cancel, &mut |entry| {
                entries.push((entry.path.clone(), entry.size_bytes, entry.modified));
            });
            entries.sort();
            entries
        };

        let before = snapshot(dir.path());
        run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        assert_eq!(snapshot(dir.path()), before, "scan() must be read-only");
    }

    #[test]
    fn two_runs_of_one_disk_produce_the_same_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0..6 {
            let bytes = payload(16 * 1024, index, None);
            write(&dir.path().join(format!("x{index}-1.bin")), &bytes);
            write(&dir.path().join(format!("x{index}-2.bin")), &bytes);
        }

        let first = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        let second = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);

        let shape = |scan: &DuplicateScan| {
            scan.sets
                .iter()
                .map(|set| {
                    (
                        set.content_hash.clone(),
                        set.wasted_bytes,
                        set.copies.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(shape(&first), shape(&second), "scans must be deterministic");
    }

    #[test]
    fn the_default_roots_are_the_profile_folders_that_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("Pictures")).expect("dir");
        fs::create_dir_all(dir.path().join("Downloads")).expect("dir");

        let env = MapEnv::new().with("USERPROFILE", dir.path().to_string_lossy());
        let roots = default_roots(&env);

        assert_eq!(roots.len(), 2, "only the folders that exist: {roots:?}");
        assert!(roots.contains(&dir.path().join("Pictures")));
        assert!(roots.contains(&dir.path().join("Downloads")));

        assert!(
            default_roots(&MapEnv::new()).is_empty(),
            "no profile means no default roots, not a guess"
        );
    }

    #[test]
    fn findings_carry_the_set_and_the_keeper_flag() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = payload(32 * 1024, 14, None);
        write(&dir.path().join("Pictures/a.bin"), &bytes);
        write(&dir.path().join("Downloads/a (1).bin"), &bytes);

        let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
        assert_eq!(scan.findings.len(), 2);

        for finding in &scan.findings {
            assert_eq!(finding.module, "duplicates");
            assert!(finding.meta.get("setId").is_some(), "got {}", finding.meta);
            assert!(finding.meta.get("contentHash").is_some());
            assert!(finding.meta.get("keeper").is_some());
        }
        assert_eq!(
            scan.findings
                .iter()
                .filter(|f| f.meta.get("keeper").and_then(|v| v.as_bool()) == Some(true))
                .count(),
            1
        );
    }

    /// Review M15: `copiesInSet` reported `Vec::capacity()`, which is a fact
    /// about the allocator and not about the set.
    ///
    /// **This test could not have failed against the old code**, and saying so
    /// is the point: `Vec::with_capacity(n)` is only guaranteed to give *at
    /// least* `n`, and in practice gives exactly `n`, so the wrong expression
    /// returned the right number on every allocator anyone was going to run. The
    /// defect was that the number was right by coincidence — not reachable from
    /// the outside, and so not something a test can reproduce.
    ///
    /// What this pins is the contract the UI reads: every finding in a set
    /// agrees about how big that set is, across several set sizes, whatever the
    /// vectors underneath do next.
    #[test]
    fn every_finding_reports_the_real_size_of_its_set() {
        for copies in [2usize, 3, 5] {
            let dir = tempfile::tempdir().expect("tempdir");
            let bytes = payload(32 * 1024, 21, None);
            for index in 0..copies {
                write(&dir.path().join(format!("Pictures/copy-{index}.bin")), &bytes);
            }

            let scan = run(dir.path(), MIN_ALLOWED_SIZE_BYTES);
            assert_eq!(scan.sets.len(), 1, "one set of {copies}");
            assert_eq!(scan.sets[0].copies.len(), copies);
            assert_eq!(scan.findings.len(), copies);

            for finding in &scan.findings {
                assert_eq!(
                    finding.meta.get("copiesInSet").and_then(|v| v.as_u64()),
                    Some(copies as u64),
                    "`{}` disagrees about how big its set is: {}",
                    finding.path.display(),
                    finding.meta
                );
            }
        }
    }

    #[test]
    fn the_manifest_is_complete_and_the_id_is_kebab_case() {
        let module = DuplicatesModule;
        let manifest = module.manifest();
        assert_eq!(manifest.id, module.id());
        assert_eq!(manifest.id, "duplicates");
        assert!(!manifest.name.trim().is_empty());
        assert!(!manifest.description.trim().is_empty());
        assert_eq!(manifest.capabilities, ModuleCapabilities::FULL);
    }

    #[test]
    fn the_wire_format_is_camel_case_for_the_webview() {
        let scan = DuplicateScan {
            sets: vec![DuplicateSet {
                id: Uuid::nil(),
                content_hash: "abc".to_owned(),
                size_bytes: 10,
                wasted_bytes: 10,
                copies: vec![DuplicateCopy {
                    finding_id: Uuid::nil(),
                    path: PathBuf::from("C:\\a.bin"),
                    created: None,
                    modified: None,
                    keeper: true,
                    keeper_score: 50,
                    keeper_reason: "because".to_owned(),
                }],
            }],
            ..DuplicateScan::default()
        };
        let json = serde_json::to_value(&scan).expect("serialize");
        assert!(json.get("linkedNames").is_some(), "got {json}");
        assert!(json.get("filesHashed").is_some(), "got {json}");
        let set = &json["sets"][0];
        assert!(set.get("contentHash").is_some(), "got {set}");
        assert!(set.get("wastedBytes").is_some(), "got {set}");
        assert!(set["copies"][0].get("keeperScore").is_some(), "got {set}");

        assert_eq!(
            serde_json::to_string(&Stage::FullHashing).expect("serialize"),
            r#""fullHashing""#
        );
    }
}
