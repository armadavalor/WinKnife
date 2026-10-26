//! Similar images (research C18).
//!
//! Two images are "similar" when their 64-bit difference hashes (dHash: the
//! picture shrunk to 9x8 grey pixels, one bit per "is the right neighbour
//! brighter") differ in at most [`SIMILAR_MAX_DISTANCE`] bits. That catches
//! resized, re-encoded and lightly edited copies, which byte-equal duplicate
//! detection cannot.
//!
//! Looking alike is not being the same, so this mode is more careful than the
//! byte-equal one: nothing is preselected (the UI's job), one image per group
//! is always kept, and gate 4 re-hashes the file so it is the image the user
//! actually looked at. Protected and locked files are shown but never offered.
//!
//! v1 reads JPEG, PNG, WebP and GIF (first frame) through the `image` crate.
//! HEIC/JXL are out (see DECISIONS-NEEDED).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::imageops::FilterType;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    folders::distinct_roots, plural, resolve_root, DuplicateContext, DuplicateProgress,
    ProgressSink, Stage,
};
#[cfg(not(windows))]
use super::is_hidden_or_system;
use crate::core::brain::Brain;
use crate::core::content_hash::full_hash_cancellable;
use crate::core::scanner::{self, WalkOptions};
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket,
};

/// Largest Hamming distance (of 64 bits) still called "similar": 87.5 %.
pub const SIMILAR_MAX_DISTANCE: u32 = 8;
/// Extensions read in v1.
const IMAGE_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "webp", "gif"];
/// Images above this many pixels are skipped: decoding one costs 4 bytes/pixel.
const MAX_PIXELS: u64 = 80_000_000;
/// Decoder allocation cap per image, on top of [`MAX_PIXELS`].
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;
/// Images decoded at once. Bounds peak memory to about this many full frames.
const DECODE_THREADS: usize = 4;
/// A hash with this few set (or unset) bits is a flat or smooth picture; every
/// such picture would look "similar" to every other one, so it is left out.
const FEATURELESS_BITS: u32 = 2;

const ATTR_HIDDEN: u32 = 0x0000_0002;
const ATTR_SYSTEM: u32 = 0x0000_0004;
const ATTR_OFFLINE: u32 = 0x0000_1000;
const ATTR_RECALL_ON_OPEN: u32 = 0x0004_0000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// What an image scan found.
#[derive(Debug, Default)]
pub struct ImageScan {
    /// One `File` finding per image that has at least one look-alike.
    pub findings: Vec<Finding>,
    /// Roots refused before walking, with the reason.
    pub skipped_roots: Vec<String>,
    /// Entries the walk could not read, plus images that would not decode.
    pub unreadable: u64,
    /// Online-only cloud files (OneDrive and the like), skipped unopened:
    /// reading one would download it.
    pub cloud_only: u64,
    /// The scan was cancelled; `findings` is empty.
    pub cancelled: bool,
}

/// One group of look-alike images, as the UI shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGroup {
    /// Stable within one scan; the key `set_similar_image_keeper` takes.
    pub id: String,
    /// Keeper first.
    pub members: Vec<ImageMember>,
}

/// One image in a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMember {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
    /// The one that stays.
    pub keeper: bool,
    /// Protected or locked: shown, never offered.
    pub protected: bool,
    /// Percent of hash bits shared with the keeper (100 for the keeper).
    pub similarity: u8,
    pub reason: String,
    pub finding_ids: Vec<Uuid>,
    pub recommendation_ids: Vec<Uuid>,
}

/* -------------------------------------------------------------------------
 * scan
 * ---------------------------------------------------------------------- */

#[derive(Debug, Clone)]
struct Hashed {
    path: PathBuf,
    size_bytes: u64,
    width: u32,
    height: u32,
    dhash: u64,
    content_hash: String,
}

/// Find groups of look-alike images under the context's roots. Read-only.
pub fn scan_images(ctx: &DuplicateContext<'_>, progress: Option<ProgressSink<'_>>) -> ImageScan {
    scan_images_above(ctx, ctx.settings.clone().validated().min_size_bytes, progress)
}

/// [`scan_images`] with the size floor given; tests go below the settings' minimum.
fn scan_images_above(
    ctx: &DuplicateContext<'_>,
    min_size: u64,
    progress: Option<ProgressSink<'_>>,
) -> ImageScan {
    let mut output = ImageScan::default();
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
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
        let options = WalkOptions {
            report_links: true,
            ..Default::default()
        };
        let summary = scanner::walk_with(&root, options, ctx.cancel, &mut |entry| {
            let candidate = !entry.is_dir
                && !entry.is_link
                && entry.size_bytes >= min_size
                && is_image(&entry.path)
                && ctx.protected.reason_for(&entry.path).is_none();
            if !candidate {
                return;
            }
            // Attributes only: reading them never recalls a cloud placeholder.
            match classify(attributes_of(&entry.path)) {
                Attributes::Ordinary => files.push((entry.path.clone(), entry.size_bytes)),
                Attributes::CloudOnly => output.cloud_only += 1,
                Attributes::HiddenOrSystem => {}
            }
        });
        output.unreadable += summary.unreadable.len() as u64;
        if summary.cancelled {
            return ImageScan {
                cancelled: true,
                ..output
            };
        }
    }

    let Some((hashed, undecodable)) = hash_images(&files, ctx, progress) else {
        return ImageScan {
            cancelled: true,
            ..output
        };
    };
    output.unreadable += undecodable;
    output.findings = emit_findings(&group_similar(&hashed), &hashed);
    output
}

#[derive(Debug, PartialEq, Eq)]
enum Attributes {
    Ordinary,
    /// Not on this disk; opening it would download it.
    CloudOnly,
    HiddenOrSystem,
}

fn classify(attributes: u32) -> Attributes {
    if attributes & (ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS) != 0 {
        Attributes::CloudOnly
    } else if attributes & (ATTR_HIDDEN | ATTR_SYSTEM) != 0 {
        Attributes::HiddenOrSystem
    } else {
        Attributes::Ordinary
    }
}

#[cfg(windows)]
fn attributes_of(path: &Path) -> u32 {
    use std::os::windows::fs::MetadataExt;
    // Unreadable metadata: let the decoder fail and count it as unreadable.
    std::fs::symlink_metadata(path).map_or(0, |metadata| metadata.file_attributes())
}

#[cfg(not(windows))]
fn attributes_of(path: &Path) -> u32 {
    if is_hidden_or_system(path) { ATTR_HIDDEN } else { 0 }
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

/// Decode and hash every image. `None` when cancelled.
fn hash_images(
    files: &[(PathBuf, u64)],
    ctx: &DuplicateContext<'_>,
    progress: Option<ProgressSink<'_>>,
) -> Option<(Vec<Hashed>, u64)> {
    let total = files.len() as u64;
    let done = AtomicU64::new(0);
    let undecodable = AtomicU64::new(0);
    let is_cancelled = || ctx.cancel.is_cancelled();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(DECODE_THREADS)
        .build()
        .ok()?;

    let hashed: Vec<Hashed> = pool.install(|| {
        files
            .par_iter()
            .filter_map(|(path, size)| {
                if is_cancelled() {
                    return None;
                }
                let result = hash_isolated(|| hash_one(path, *size, &is_cancelled));
                let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(report) = progress {
                    report(&DuplicateProgress {
                        stage: Stage::FullHashing,
                        done: finished,
                        total,
                        current_path: path.clone(),
                    });
                }
                match result {
                    Ok(hashed) => hashed,
                    Err(err) => {
                        undecodable.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(path = %path.display(), error = %err, "image skipped");
                        None
                    }
                }
            })
            .collect()
    });
    if is_cancelled() {
        return None;
    }
    Some((hashed, undecodable.load(Ordering::Relaxed)))
}

/// Runs one file's hashing so a decoder panic on a crafted file costs that file,
/// not the scan.
fn hash_isolated(
    hash: impl FnOnce() -> Result<Option<Hashed>, String>,
) -> Result<Option<Hashed>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(hash))
        .unwrap_or_else(|_| Err("the image decoder panicked".to_owned()))
}

/// `Ok(None)` for an image that is fine but not worth comparing (featureless,
/// or cancelled mid-read).
fn hash_one(
    path: &Path,
    size_bytes: u64,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Option<Hashed>, String> {
    let (width, height, dhash) = perceptual_hash(path)?;
    if !(FEATURELESS_BITS..=64 - FEATURELESS_BITS).contains(&dhash.count_ones()) {
        return Ok(None);
    }
    let content_hash = match full_hash_cancellable(path, &is_cancelled) {
        Ok(hash) => hash,
        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    Ok(Some(Hashed {
        path: path.to_path_buf(),
        size_bytes,
        width,
        height,
        dhash,
        content_hash,
    }))
}

/// Header-checked decode, then the 64-bit dHash. Opens the file read-only.
fn perceptual_hash(path: &Path) -> Result<(u32, u32, u64), String> {
    let open = || {
        image::ImageReader::open(path)
            .and_then(|reader| reader.with_guessed_format())
            .map_err(|err| err.to_string())
    };
    let (width, height) = open()?.into_dimensions().map_err(|err| err.to_string())?;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("{width}x{height} is larger than this scan decodes"));
    }
    let mut reader = open()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let picture = reader.decode().map_err(|err| err.to_string())?;
    let small = picture.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    Ok((width, height, dhash_of(&small)))
}

/// One bit per pixel pair: set when the right neighbour is brighter.
fn dhash_of(small: &image::GrayImage) -> u64 {
    (0..8u32)
        .flat_map(|y| (0..8u32).map(move |x| (x, y)))
        .fold(0u64, |bits, (x, y)| {
            let brighter = small.get_pixel(x + 1, y)[0] > small.get_pixel(x, y)[0];
            (bits << 1) | u64::from(brighter)
        })
}

fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Percent of the 64 bits two hashes share.
fn similarity(a: u64, b: u64) -> u8 {
    (100 - distance(a, b) * 100 / 64) as u8
}

/// Index groups (2+ members) of images within [`SIMILAR_MAX_DISTANCE`].
///
/// shortcut: all-pairs O(n²) with single linkage, fine to ~50k images; a
/// BK-tree is the upgrade if people scan bigger libraries. Single linkage can
/// chain A~B~C with A and C further apart; the UI shows each member's
/// similarity to the keeper, so that is visible rather than hidden.
fn group_similar(hashed: &[Hashed]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..hashed.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..hashed.len() {
        for j in i + 1..hashed.len() {
            if distance(hashed[i].dhash, hashed[j].dhash) <= SIMILAR_MAX_DISTANCE {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..hashed.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    groups.into_values().filter(|g| g.len() > 1).collect()
}

fn emit_findings(groups: &[Vec<usize>], hashed: &[Hashed]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for members in groups {
        let mut hashes: Vec<&str> = members.iter().map(|&i| hashed[i].content_hash.as_str()).collect();
        hashes.sort_unstable();
        let id = blake3::hash(hashes.join("\n").as_bytes()).to_hex()[..16].to_owned();
        for &i in members {
            let image = &hashed[i];
            let mut finding = Finding::new("duplicates", image.path.clone(), FindingKind::File);
            finding.size_bytes = image.size_bytes;
            finding.meta = serde_json::json!({
                "imageGroupId": id,
                "width": image.width,
                "height": image.height,
                "dhash": format!("{:016x}", image.dhash),
                "contentHash": image.content_hash,
            });
            findings.push(finding);
        }
    }
    findings
}

/* -------------------------------------------------------------------------
 * analyze
 * ---------------------------------------------------------------------- */

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageMeta {
    image_group_id: String,
    width: u32,
    height: u32,
    dhash: String,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct Draft {
    finding_id: Uuid,
    path: PathBuf,
    size_bytes: u64,
    meta: ImageMeta,
    dhash: u64,
}

/// Whether a finding came from this mode.
pub fn is_image_finding(finding: &Finding) -> bool {
    finding.meta.get("imageGroupId").is_some()
}

/// Turn image findings into groups and recommendations. Never Safe, and the
/// keeper (default: most pixels, then largest file) is never offered.
pub fn analyze(
    findings: &[Finding],
    brain: &Brain,
    keepers: &HashMap<String, PathBuf>,
) -> (Vec<ImageGroup>, Vec<Recommendation>) {
    let mut drafts: BTreeMap<String, Vec<Draft>> = BTreeMap::new();
    for finding in findings {
        let Ok(meta) = serde_json::from_value::<ImageMeta>(finding.meta.clone()) else {
            continue;
        };
        let Ok(dhash) = u64::from_str_radix(&meta.dhash, 16) else {
            continue;
        };
        drafts.entry(meta.image_group_id.clone()).or_default().push(Draft {
            finding_id: finding.id,
            path: finding.path.clone(),
            size_bytes: finding.size_bytes,
            meta,
            dhash,
        });
    }

    let mut groups = Vec::new();
    let mut recommendations = Vec::new();
    for (id, members) in drafts {
        if members.len() < 2 {
            continue;
        }
        let (group, recs) = analyze_group(id.clone(), members, brain, keepers.get(&id));
        groups.push(group);
        recommendations.extend(recs);
    }
    (groups, recommendations)
}

fn pick_keeper(members: &[Draft], preferred: Option<&PathBuf>) -> usize {
    let chosen = preferred.and_then(|want| {
        members
            .iter()
            .position(|m| crate::core::path_compare::same_path(want, &m.path))
    });
    chosen.unwrap_or_else(|| {
        members
            .iter()
            .enumerate()
            .max_by_key(|(_, m)| {
                (
                    u64::from(m.meta.width) * u64::from(m.meta.height),
                    m.size_bytes,
                    std::cmp::Reverse(m.path.clone()),
                )
            })
            .map_or(0, |(i, _)| i)
    })
}

fn analyze_group(
    id: String,
    members: Vec<Draft>,
    brain: &Brain,
    preferred: Option<&PathBuf>,
) -> (ImageGroup, Vec<Recommendation>) {
    let keeper = pick_keeper(&members, preferred);
    let keeper_path = members[keeper].path.clone();
    let keep_reason = if preferred.is_some_and(|p| crate::core::path_compare::same_path(p, &keeper_path)) {
        "Kept because you chose it.".to_owned()
    } else {
        "Kept because it has the most pixels.".to_owned()
    };
    let others = plural(members.len() - 1, "image looks", "images look");

    let mut out = Vec::new();
    let mut recommendations = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let similar = similarity(member.dhash, members[keeper].dhash);
        let refusal = brain.path_refusal(&member.path).map(|r| r.reason).or_else(|| {
            let lock = brain.lock_state(&member.path);
            (!lock.is_free()).then(|| format!("left alone: {}", lock.reason()))
        });
        let (risk, action, reason) = match (&refusal, index == keeper) {
            (Some(why), _) => (RiskBucket::DoNotDelete, Action::InfoOnly, why.clone()),
            (None, true) => (
                RiskBucket::NeedsReview,
                Action::InfoOnly,
                format!("{keep_reason} {others} like it."),
            ),
            (None, false) => (
                RiskBucket::NeedsReview,
                Action::Quarantine,
                format!(
                    "Looks {similar}% like `{}` ({}x{}), which is kept. Similar is not \
                     identical, so compare them first; it goes to quarantine and can be restored.",
                    keeper_path.display(),
                    members[keeper].meta.width,
                    members[keeper].meta.height,
                ),
            ),
        };
        let rec = Recommendation::new(
            vec![member.finding_id],
            risk,
            similar.min(100),
            action,
            reason.clone(),
            if action.is_mutating() { member.size_bytes } else { 0 },
            Justification::SimilarImage {
                content_hash: member.meta.content_hash.clone(),
                keeper_path: keeper_path.clone(),
            },
        )
        .map_err(|err| {
            tracing::error!(path = %member.path.display(), error = %err, "dropped a similar-image recommendation");
        })
        .ok();
        out.push(ImageMember {
            path: member.path.clone(),
            width: member.meta.width,
            height: member.meta.height,
            size_bytes: member.size_bytes,
            keeper: index == keeper,
            protected: refusal.is_some(),
            similarity: similar,
            reason,
            finding_ids: vec![member.finding_id],
            recommendation_ids: rec.iter().map(|r| r.id).collect(),
        });
        recommendations.extend(rec);
    }
    out.sort_by_key(|m| !m.keeper);
    (ImageGroup { id, members: out }, recommendations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::brain::ProtectedPaths;
    use crate::core::platform::{AlwaysUnlocked, StubProcessProbe};
    use crate::core::scanner::CancellationToken;
    use crate::modules::duplicates::DuplicatesSettings;
    use image::{Rgb, RgbImage};
    use std::sync::Arc;

    /// A diagonal gradient with a bright square at `(sx, sy)`: real structure.
    fn picture(width: u32, height: u32, sx: u32, sy: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| {
            let in_square = (sx..sx + width / 4).contains(&x) && (sy..sy + height / 4).contains(&y);
            let base = ((x * 255 / width) / 2 + (y * 255 / height) / 4) as u8;
            if in_square { Rgb([250, 250, 250]) } else { Rgb([base, base / 2, 40]) }
        })
    }

    fn scan(root: &Path, protected: &ProtectedPaths) -> ImageScan {
        let settings = DuplicatesSettings { min_size_bytes: 1, roots: Vec::new() };
        let cancel = CancellationToken::new();
        scan_images_above(
            &DuplicateContext {
                roots: std::slice::from_ref(&root.to_path_buf()),
                protected,
                settings: &settings,
                cancel: &cancel,
            },
            0,
            None,
        )
    }

    fn brain_with(protected: ProtectedPaths) -> Brain {
        Brain::builder()
            .lock_probe(Arc::new(AlwaysUnlocked))
            .process_probe(Arc::new(StubProcessProbe::none()))
            .protected_paths(protected)
            .build()
    }

    /// big.png and its half-size JPEG look alike; other.png does not.
    fn library(dir: &Path) {
        picture(256, 192, 20, 20).save(dir.join("big.png")).expect("png");
        image::imageops::resize(&picture(256, 192, 20, 20), 128, 96, FilterType::Triangle)
            .save(dir.join("small.jpg"))
            .expect("jpg");
        picture(256, 192, 180, 130).save(dir.join("other.png")).expect("png");
    }

    #[test]
    fn a_resized_copy_is_grouped_and_a_different_picture_is_not() {
        let tmp = tempfile::tempdir().expect("tmp");
        library(tmp.path());
        let result = scan(tmp.path(), &ProtectedPaths::empty());
        let names: Vec<String> = result
            .findings
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(result.findings.len(), 2, "{names:?}");
        assert!(names.contains(&"big.png".to_owned()) && names.contains(&"small.jpg".to_owned()));
    }

    #[test]
    fn the_largest_image_is_kept_and_never_offered() {
        let tmp = tempfile::tempdir().expect("tmp");
        library(tmp.path());
        let result = scan(tmp.path(), &ProtectedPaths::empty());
        let (groups, recs) = analyze(&result.findings, &brain_with(ProtectedPaths::empty()), &HashMap::new());
        assert_eq!(groups.len(), 1);
        let keeper = &groups[0].members[0];
        assert!(keeper.keeper && keeper.path.ends_with("big.png"));
        let movers: Vec<_> = recs.iter().filter(|r| r.action == Action::Quarantine).collect();
        assert_eq!(movers.len(), 1, "exactly the smaller copy is offered");
        assert!(movers[0].risk != RiskBucket::Safe, "look-alikes are never Safe");
        let Justification::SimilarImage { keeper_path, .. } = &movers[0].justification else {
            panic!("wrong justification");
        };
        assert!(keeper_path.ends_with("big.png"));
        assert!(movers[0].reason.contains('%'), "the reason shows the similarity");
    }

    #[test]
    fn choosing_another_keeper_swaps_who_is_offered() {
        let tmp = tempfile::tempdir().expect("tmp");
        library(tmp.path());
        let result = scan(tmp.path(), &ProtectedPaths::empty());
        let group_id = result.findings[0].meta["imageGroupId"].as_str().unwrap().to_owned();
        let keepers = HashMap::from([(group_id, tmp.path().join("small.jpg"))]);
        let (groups, recs) = analyze(&result.findings, &brain_with(ProtectedPaths::empty()), &keepers);
        assert!(groups[0].members[0].path.ends_with("small.jpg"));
        let mover = recs.iter().find(|r| r.action == Action::Quarantine).expect("one mover");
        let path = &result.findings.iter().find(|f| f.id == mover.finding_ids[0]).unwrap().path;
        assert!(path.ends_with("big.png"));
    }

    #[test]
    fn every_group_keeps_at_least_one_image_even_when_all_are_protected() {
        let tmp = tempfile::tempdir().expect("tmp");
        library(tmp.path());
        let result = scan(tmp.path(), &ProtectedPaths::empty());
        // Protected after the scan: the brain refuses at analyze time too.
        let protected = ProtectedPaths::from_trees(vec![tmp.path().to_path_buf()]);
        let (groups, recs) = analyze(&result.findings, &brain_with(protected), &HashMap::new());
        assert!(groups[0].members.iter().all(|m| m.protected));
        assert!(recs.iter().all(|r| !r.action.is_mutating()), "protected images are never offered");
        let (_, recs) = analyze(&result.findings, &brain_with(ProtectedPaths::empty()), &HashMap::new());
        assert!(recs.iter().any(|r| !r.action.is_mutating()), "one image always stays");
    }

    #[test]
    fn protected_folders_are_not_walked() {
        let tmp = tempfile::tempdir().expect("tmp");
        library(tmp.path());
        let protected = ProtectedPaths::from_trees(vec![tmp.path().to_path_buf()]);
        let result = scan(tmp.path(), &protected);
        assert!(result.findings.is_empty());
        assert_eq!(result.skipped_roots.len(), 1);
    }

    #[test]
    fn flat_pictures_are_not_all_similar_to_each_other() {
        let tmp = tempfile::tempdir().expect("tmp");
        RgbImage::from_pixel(64, 64, Rgb([255, 0, 0])).save(tmp.path().join("red.png")).unwrap();
        RgbImage::from_pixel(64, 64, Rgb([0, 0, 255])).save(tmp.path().join("blue.png")).unwrap();
        assert!(scan(tmp.path(), &ProtectedPaths::empty()).findings.is_empty());
    }

    #[test]
    fn a_cancelled_scan_reports_nothing() {
        let tmp = tempfile::tempdir().expect("tmp");
        library(tmp.path());
        let settings = DuplicatesSettings { min_size_bytes: 1, roots: Vec::new() };
        let cancel = CancellationToken::new();
        cancel.cancel();
        let protected = ProtectedPaths::empty();
        let result = scan_images(
            &DuplicateContext {
                roots: std::slice::from_ref(&tmp.path().to_path_buf()),
                protected: &protected,
                settings: &settings,
                cancel: &cancel,
            },
            None,
        );
        assert!(result.cancelled && result.findings.is_empty());
    }

    #[test]
    fn images_below_the_size_floor_are_not_read() {
        let tmp = tempfile::tempdir().expect("tmp");
        library(tmp.path());
        let settings = DuplicatesSettings { min_size_bytes: 1, roots: Vec::new() };
        let cancel = CancellationToken::new();
        let protected = ProtectedPaths::empty();
        let result = scan_images_above(
            &DuplicateContext {
                roots: std::slice::from_ref(&tmp.path().to_path_buf()),
                protected: &protected,
                settings: &settings,
                cancel: &cancel,
            },
            u64::MAX,
            None,
        );
        assert!(result.findings.is_empty() && result.unreadable == 0);
    }

    #[test]
    fn cloud_placeholders_are_skipped_before_hidden_files() {
        assert_eq!(classify(0x20), Attributes::Ordinary, "archive bit only");
        assert_eq!(classify(ATTR_RECALL_ON_DATA_ACCESS | 0x20), Attributes::CloudOnly);
        assert_eq!(classify(ATTR_RECALL_ON_OPEN), Attributes::CloudOnly);
        assert_eq!(classify(ATTR_OFFLINE), Attributes::CloudOnly);
        assert_eq!(classify(ATTR_HIDDEN | ATTR_RECALL_ON_DATA_ACCESS), Attributes::CloudOnly);
        assert_eq!(classify(ATTR_SYSTEM), Attributes::HiddenOrSystem);
    }

    #[test]
    fn a_decoder_panic_costs_one_file_not_the_scan() {
        let result = hash_isolated(|| panic!("crafted file"));
        assert!(result.is_err());
    }

    #[test]
    fn similarity_is_a_percentage_of_shared_bits() {
        assert_eq!(similarity(0, 0), 100);
        assert_eq!(similarity(0, 0xFF), 88);
        assert_eq!(similarity(0, u64::MAX), 0);
    }
}
