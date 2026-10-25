//! Treemap: answer "where did my disk space go?" (`docs/modules/04-treemap.md`).
//!
//! # The shape of the answer
//!
//! Module spec 04 offers two ways to get a tree to the UI and says to pick one:
//! stream `treemap:node` events, or serve a paginated `get_children(path)`. The
//! recommendation is `get_children`, and that is what this is — there is no
//! streaming path, deliberately, because two ways to build the same picture is
//! two pictures that can disagree.
//!
//! So the work splits in two:
//!
//! - [`scan`] walks a drive once and aggregates every directory's size
//!   bottom-up. What it keeps is **directories only**: one row per folder,
//!   not per file. A drive with two million files has perhaps fifty thousand
//!   folders, so the model stays small enough to hold in memory *and* to write
//!   into SQLite as a per-drive cache.
//! - [`children`] answers one directory at a time. It reads that single
//!   directory from disk for its immediate files (one `read_dir`, microseconds)
//!   and takes each subdirectory's *total* from the aggregate the scan built.
//!   Zooming in is therefore cheap and always shows current file names, while
//!   the big numbers come from the scan.
//!
//! # Four things that would otherwise be lies
//!
//! 1. **Reparse points are drawn but never counted.** A junction pointing at a
//!    40 GB folder would make those bytes appear twice. The walk reports them
//!    (`WalkOptions::report_links`) with a size of zero and never descends, so
//!    the picture shows the link exists without double-counting it.
//! 2. **A directory we cannot read is marked, not zeroed.** A silent zero is a
//!    lie about where space went, and permission-denied folders are ordinary on
//!    Windows. Their size is reported as unknown and the ancestors that contain
//!    them say how many they hide.
//! 3. **Sizes are logical, not on-disk.** NTFS compression and sparse files make
//!    the two differ, sometimes hugely. The scan counts what the files contain
//!    and reports how many of them are compressed, so the UI can say so rather
//!    than quietly disagreeing with the drive's free-space figure.
//! 4. **A cached tree says how old it is.** The header row carries the scan
//!    time; the UI shows it next to the rescan button.
//!
//! # Read-only
//!
//! Nothing in this file writes to the filesystem. `scan` and `children` walk and
//! read directory entries; the only writes are to the app's own SQLite cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::core::brain::Brain;
use crate::core::db::{with_db, DbAccess};
use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::path_compare::CanonicalPath;
use crate::core::scanner::{self, CancellationToken, WalkOptions};
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket,
};

/// Module id, also the `module` of every finding [`propose`] makes.
pub const MODULE_ID: &str = "treemap";

/// How many children one `children` call returns before the tail is folded into
/// a single "everything else" row.
///
/// Module spec 04's non-goal is "no file-level rects below 0.1% of view area";
/// 0.1% of a view is at most a thousand rectangles, and past a few hundred a
/// treemap is a texture rather than a picture. Whatever is left is returned as
/// one aggregate row so the rectangles still add up to the parent.
pub const MAX_CHILDREN: usize = 400;

/// What a file is, for colouring.
///
/// Extension-based, which is what a treemap needs: the question is "what kind of
/// thing is eating the disk", and that is answered well enough by the name. No
/// file is opened to decide this — the scan stays read-only, including
/// last-access times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    /// A folder.
    Directory,
    /// Films and clips.
    Video,
    /// Photos and graphics.
    Image,
    /// Music and recordings.
    Audio,
    /// Zip, 7z, iso, and friends.
    Archive,
    /// Source code and text.
    Code,
    /// Programs, libraries and installers.
    App,
    /// Documents and spreadsheets.
    Document,
    /// Everything else.
    Other,
}

impl Category {
    /// Classify by extension.
    pub fn of(path: &Path) -> Self {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return Self::Other;
        };
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
            // `.ts` is both a transport stream and TypeScript. Code wins: this
            // app's users are far likelier to have a repository than a raw DVB
            // capture, and a mislabelled colour is a smaller lie than a
            // mislabelled one in the other direction would be for a developer
            // looking at their own disk.
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
            | "vob" => Self::Video,
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "heic" | "raw"
            | "cr2" | "nef" | "svg" | "ico" | "psd" => Self::Image,
            "mp3" | "flac" | "wav" | "aac" | "ogg" | "wma" | "m4a" | "opus" | "aiff" => Self::Audio,
            "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "iso" | "cab" | "wim" | "vhd"
            | "vhdx" => Self::Archive,
            "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "java" | "c" | "h" | "cpp" | "hpp"
            | "cs" | "go" | "rb" | "php" | "swift" | "kt" | "sh" | "ps1" | "json" | "toml"
            | "yaml" | "yml" | "xml" | "html" | "css" | "sql" | "md" => Self::Code,
            "exe" | "dll" | "msi" | "sys" | "so" | "dylib" | "pdb" | "lib" | "obj" | "appx"
            | "msix" | "bin" => Self::App,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
            | "txt" | "rtf" | "csv" | "epub" | "mobi" => Self::Document,
            _ => Self::Other,
        }
    }
}

/* -------------------------------------------------------------------------
 * The aggregate tree
 * ---------------------------------------------------------------------- */

/// One directory's aggregated numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirAggregate {
    /// Absolute path of the directory.
    pub path: PathBuf,
    /// Everything beneath it, including its own files.
    pub total_bytes: u64,
    /// Just the files directly inside it.
    pub own_bytes: u64,
    /// Files beneath it, at every depth.
    pub file_count: u64,
    /// Directories beneath it, at every depth.
    pub dir_count: u64,
    /// Whether this directory itself could be listed.
    ///
    /// `false` means its size is a floor rather than a figure — see
    /// [`DirAggregate::sized`].
    pub readable: bool,
    /// Directories beneath it that could not be listed.
    pub unreadable_below: u64,
    /// NTFS-compressed files beneath it, so the UI can note that the logical
    /// figure is larger than the space actually used.
    pub compressed_below: u64,
}

impl DirAggregate {
    /// Whether `total_bytes` is the whole truth.
    ///
    /// A directory with unreadable descendants holds at least this much and
    /// possibly more. The UI must never present a floor as a total.
    pub fn sized(&self) -> bool {
        self.readable && self.unreadable_below == 0
    }
}

/// What one scan of a drive produced.
#[derive(Debug, Clone, Default)]
pub struct TreeScan {
    /// Root that was walked, e.g. `C:\`.
    pub root: PathBuf,
    /// Every directory, root first.
    pub dirs: Vec<DirAggregate>,
    /// Reparse points seen and deliberately not counted.
    pub links: Vec<PathBuf>,
    /// Directories the walk could not list.
    pub unreadable: Vec<PathBuf>,
    /// Whether the walk stopped early.
    pub cancelled: bool,
    /// When the walk finished.
    pub scanned_at: DateTime<Utc>,
}

impl TreeScan {
    /// The root's aggregate, when the walk produced one.
    pub fn root_total(&self) -> Option<&DirAggregate> {
        self.dirs.first()
    }
}

/// Progress while a drive is being walked.
#[derive(Debug, Clone)]
pub struct TreeProgress<'a> {
    /// Entries seen so far.
    pub entries: u64,
    /// Where the walk is now.
    pub current_path: &'a Path,
}

/// Walk `root` and aggregate every directory's size bottom-up.
///
/// Read-only. Never follows a reparse point and never descends outside `root`.
pub fn scan(
    root: &Path,
    cancel: &CancellationToken,
    mut progress: Option<&mut dyn FnMut(&TreeProgress<'_>)>,
) -> TreeScan {
    let mut arena = DirArena::new(root);
    let mut links = Vec::new();

    let mut seen = 0u64;
    let summary = scanner::walk_with(
        root,
        WalkOptions {
            report_links: true,
            // The treemap reads no clocks: every figure it shows is a size, and
            // it accepts that the listing's copy of a size can lag. Skipping the
            // authoritative stat is what makes a whole-drive walk affordable
            // (see `core::scanner`'s header for the measurement).
            accurate_timestamps: false,
        },
        cancel,
        &mut |entry| {
            seen += 1;
            if let Some(sink) = progress.as_deref_mut() {
                sink(&TreeProgress {
                    entries: seen,
                    current_path: &entry.path,
                });
            }

            // A reparse point is drawn but never counted: its target's bytes
            // belong to wherever the target really lives, and adding them here
            // would make one folder's contents appear on the map twice.
            if entry.is_link {
                links.push(entry.path.clone());
                return;
            }

            if entry.is_dir {
                arena.ensure(&entry.path);
                return;
            }

            let Some(parent) = entry.path.parent() else {
                return;
            };
            let slot = arena.ensure(parent);
            let node = &mut arena.dirs[slot];
            node.own_bytes = node.own_bytes.saturating_add(entry.size_bytes);
            node.total_bytes = node.total_bytes.saturating_add(entry.size_bytes);
            node.file_count += 1;
            // From the directory listing the walk already read, never a second
            // syscall: this is asked of every file on a drive.
            if entry.compressed {
                node.compressed_below += 1;
            }
        },
    );

    // A path the walk could not read is marked rather than silently sized at
    // zero. `unreadable` usually names a directory that is already a node (its
    // parent listed it) — a file whose metadata call failed, or a directory
    // nothing listed, is instead counted as a hole in its nearest known
    // ancestor.
    for missing in &summary.unreadable {
        match arena.find(missing) {
            Some(slot) => arena.dirs[slot].readable = false,
            None => {
                if let Some(slot) = arena.nearest_ancestor(missing) {
                    arena.dirs[slot].unreadable_below += 1;
                }
            }
        }
    }

    let DirArena {
        mut dirs,
        parents,
        depths,
        ..
    } = arena;
    aggregate_bottom_up(&mut dirs, &parents, &depths);

    TreeScan {
        root: root.to_path_buf(),
        dirs,
        links,
        unreadable: summary.unreadable,
        cancelled: summary.cancelled,
        scanned_at: Utc::now(),
    }
}

/// The comparison key for a whole directory path.
///
/// Used by the two oracles, which look one arbitrary path up at a time. The
/// *scan* does not use it — see [`DirArena`] for why keying an arena on whole
/// paths is quadratic.
fn key_of(path: &Path) -> String {
    // `lexical` rather than `resolved`: resolving would be a syscall per lookup.
    // The walk hands back paths it built itself by joining onto one root, so
    // they are already one consistent spelling; the canonicalisation that
    // matters (case, separators, trailing dots) is what `lexical` does.
    CanonicalPath::lexical(path).key()
}

/// The directories a scan has found, and the index that finds them again.
///
/// # Why it is keyed the way it is
///
/// The index used to be `full canonical path string -> slot`, which is O(depth)
/// to build *and* O(depth) to hash, for every directory — so indexing a tree
/// cost O(depth²). Worse, a miss then asked `same_path` and `is_inside`, each of
/// which canonicalises through the filesystem. Measured on a 3 000-deep tree:
/// the walk took 5.34 s and `treemap::scan` around it took **75.3 s**
/// (security review M21).
///
/// So a directory is keyed on **(its parent's slot, one normalised name)**. The
/// parent is nearly always the node resolved a moment ago — the walk is
/// depth-first and sorted, so entries arrive either as a run of siblings or as a
/// descent — and the two-entry memo below turns that into one hash lookup of one
/// short string. Nothing is canonicalised through the filesystem at all: a path
/// the walk produced was built by joining onto `root`, so `strip_prefix` is
/// exact rather than a comparison, and the case/trailing-dot folding that the
/// standing rule about two spellings of one directory asks for is exactly what
/// [`crate::core::path_compare::normalise_component`] does.
#[derive(Debug)]
struct DirArena {
    root: PathBuf,
    dirs: Vec<DirAggregate>,
    parents: Vec<Option<usize>>,
    depths: Vec<u32>,
    /// `(parent slot, normalised name) -> slot`.
    children: HashMap<(usize, String), usize>,
    /// The root's slot, created on first use so a walk that saw nothing still
    /// produces an empty tree rather than a zeroed one.
    root_slot: Option<usize>,
    /// Directories with no parent inside the tree, other than the root. Keyed on
    /// the whole path because there is nothing to hang them off; there is
    /// normally none of these at all.
    detached: HashMap<String, usize>,
    /// The last few directories resolved, newest first.
    ///
    /// A ring rather than a single entry because a sorted listing interleaves
    /// files and subdirectories: `a.txt`, `bin/`, `c.txt` asks for *this*
    /// directory, then for `bin`, then for this directory again. One slot would
    /// miss on every such alternation and fall back to descending from the root,
    /// which is the O(depth) path this whole structure exists to avoid.
    recent: Vec<(PathBuf, usize)>,
}

/// How many resolved directories [`DirArena`] remembers. Four covers a listing
/// that alternates between files and subdirectories at any depth; the lookup is
/// a handful of path comparisons, which allocate nothing.
const RECENT_SLOTS: usize = 4;

impl DirArena {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            dirs: Vec::new(),
            parents: Vec::new(),
            depths: Vec::new(),
            children: HashMap::new(),
            root_slot: None,
            detached: HashMap::new(),
            recent: Vec::with_capacity(RECENT_SLOTS),
        }
    }

    /// Find or create the node for `path`, wiring it to its parent.
    fn ensure(&mut self, path: &Path) -> usize {
        if let Some(slot) = self.recent_slot_of(path) {
            return slot;
        }
        let slot = self.resolve(path);
        self.remember(path, slot);
        slot
    }

    fn resolve(&mut self, path: &Path) -> usize {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            // Not under the root at all. The walk never produces one of these —
            // it refuses to leave the tree — so this is a caller doing something
            // unusual, and it gets its own parentless node rather than a guess
            // about where it belongs.
            return self.detached_slot(path);
        };
        if relative.as_os_str().is_empty() {
            return self.root_slot();
        }

        // The common case by an enormous margin: the parent was resolved a
        // moment ago, so this is one hash lookup on one short name.
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            if let Some(parent_slot) = self.recent_slot_of(parent) {
                return self.child(parent_slot, parent, name);
            }
        }

        // Otherwise walk down from the root, creating whatever is missing. This
        // also fills in a directory the walk never listed directly.
        let mut slot = self.root_slot();
        let mut here = self.root.clone();
        for part in relative.components() {
            let std::path::Component::Normal(name) = part else {
                // `.`, `..` or a prefix cannot appear in a path the walk built
                // by joining onto the root, and descending through one would be
                // guessing about where it lands.
                return self.detached_slot(path);
            };
            let parent = std::mem::take(&mut here);
            slot = self.child(slot, &parent, name);
            here = parent.join(name);
            // Each level of the descent, against *its own* slot. Remembering the
            // parent path against the child's slot instead made `SUB` a child of
            // `Sub` rather than the same node.
            self.remember(&here, slot);
        }
        slot
    }

    /// The slot for `name` under `parent_slot`, created if this is the first
    /// time it has been seen. `parent_path` is only used to build the child's
    /// own path when one is created.
    fn child(&mut self, parent_slot: usize, parent_path: &Path, name: &std::ffi::OsStr) -> usize {
        let key = (parent_slot, crate::core::path_compare::normalise_component(name));
        if let Some(slot) = self.children.get(&key).copied() {
            return slot;
        }
        let slot = self.push(parent_path.join(name), Some(parent_slot));
        self.children.insert(key, slot);
        slot
    }

    /// The slot of `path`, if the ring already knows it. A hit moves the entry
    /// to the front, so a directory being read stays hot however deep it is.
    fn recent_slot_of(&mut self, path: &Path) -> Option<usize> {
        let found = self.recent.iter().position(|(known, _)| known == path)?;
        let entry = self.recent.remove(found);
        let slot = entry.1;
        self.recent.insert(0, entry);
        Some(slot)
    }

    fn remember(&mut self, path: &Path, slot: usize) {
        if self.recent.iter().any(|(known, _)| known == path) {
            return;
        }
        if self.recent.len() == RECENT_SLOTS {
            self.recent.pop();
        }
        self.recent.insert(0, (path.to_path_buf(), slot));
    }

    fn root_slot(&mut self) -> usize {
        match self.root_slot {
            Some(slot) => slot,
            None => {
                let slot = self.push(self.root.clone(), None);
                self.root_slot = Some(slot);
                slot
            }
        }
    }

    fn detached_slot(&mut self, path: &Path) -> usize {
        let key = key_of(path);
        if let Some(slot) = self.detached.get(&key) {
            return *slot;
        }
        let slot = self.push(path.to_path_buf(), None);
        self.detached.insert(key, slot);
        slot
    }

    fn push(&mut self, path: PathBuf, parent: Option<usize>) -> usize {
        let slot = self.dirs.len();
        self.dirs.push(DirAggregate {
            path,
            total_bytes: 0,
            own_bytes: 0,
            file_count: 0,
            dir_count: 0,
            readable: true,
            unreadable_below: 0,
            compressed_below: 0,
        });
        let depth = parent.map_or(0, |p| self.depths[p] + 1);
        self.depths.push(depth);
        self.parents.push(parent);
        slot
    }

    /// The slot for `path`, without creating one.
    fn find(&self, path: &Path) -> Option<usize> {
        let relative = path.strip_prefix(&self.root).ok()?;
        let mut slot = self.root_slot?;
        for part in relative.components() {
            let std::path::Component::Normal(name) = part else {
                return None;
            };
            let key = crate::core::path_compare::normalise_component(name);
            slot = *self.children.get(&(slot, key))?;
        }
        Some(slot)
    }

    /// The deepest known directory that contains `path`.
    fn nearest_ancestor(&self, path: &Path) -> Option<usize> {
        let mut current = path.parent();
        while let Some(dir) = current {
            if let Some(slot) = self.find(dir) {
                return Some(slot);
            }
            current = dir.parent();
        }
        None
    }
}

/// Roll every directory's numbers into its parent, deepest first.
///
/// Ordering by depth rather than trusting the walk order: a node's parent is
/// created before it, but "created before" is not "shallower than" for a node
/// the walk filled in on demand. Sorting by depth makes the pass correct
/// whatever order the arena ended up in — and it is the aggregation the module
/// spec calls bottom-up, done in one linear sweep rather than by recursion.
fn aggregate_bottom_up(dirs: &mut [DirAggregate], parents: &[Option<usize>], depths: &[u32]) {
    let mut order: Vec<usize> = (0..dirs.len()).collect();
    order.sort_by_key(|slot| std::cmp::Reverse(depths[*slot]));

    for slot in order {
        let Some(parent) = parents[slot] else { continue };
        let (total, files, dirs_below, unreadable, compressed, readable) = {
            let node = &dirs[slot];
            (
                node.total_bytes,
                node.file_count,
                node.dir_count,
                node.unreadable_below,
                node.compressed_below,
                node.readable,
            )
        };
        let up = &mut dirs[parent];
        up.total_bytes = up.total_bytes.saturating_add(total);
        up.file_count += files;
        up.dir_count += dirs_below + 1;
        up.compressed_below += compressed;
        // A folder we could not open counts once as a hole, and every hole below
        // it is already counted in `unreadable_below`.
        up.unreadable_below += unreadable + u64::from(!readable);
    }
}

/* -------------------------------------------------------------------------
 * Serving one directory at a time
 * ---------------------------------------------------------------------- */

/// One rectangle the UI will draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeChild {
    /// Absolute path, or an empty string for the aggregate tail row.
    pub path: PathBuf,
    /// What to label it.
    pub name: String,
    /// Bytes. For a directory this is its aggregated total.
    pub size_bytes: u64,
    /// Whether `size_bytes` is the whole truth, or a floor because something
    /// below could not be read.
    pub sized: bool,
    /// Whether it can be zoomed into.
    pub is_dir: bool,
    /// Whether it is a reparse point, drawn but never counted.
    pub is_link: bool,
    /// Colour class.
    pub category: Category,
    /// How many items this row stands for. `1` except for the folded tail.
    pub item_count: u64,
}

/// What `children` hands the UI for one directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeLevel {
    /// The directory this level describes.
    pub path: PathBuf,
    /// Its aggregated total, when a scan has produced one.
    pub total_bytes: u64,
    /// Whether that total is the whole truth.
    pub sized: bool,
    /// Whether this directory itself could be listed just now.
    pub readable: bool,
    /// Rows to draw, largest first.
    pub children: Vec<TreeChild>,
    /// Rows folded into the tail row, if any.
    pub folded: u64,
    /// Directories below this one that could not be listed.
    pub unreadable_below: u64,
    /// Compressed or sparse files below this one; their logical size is larger
    /// than the space they occupy.
    pub compressed_below: u64,
    /// Breadcrumb from the scan root down to here, root first.
    pub breadcrumb: Vec<PathBuf>,
}

/// Directory totals the level builder can consult.
///
/// A trait so the same code serves a freshly-scanned tree held in memory and one
/// read back out of the cache, without either learning about the other.
pub trait SizeOracle {
    /// The aggregate for `path`, when the last scan covered it.
    fn aggregate(&self, path: &Path) -> Option<DirAggregate>;
}

/// The in-memory tree a scan just produced.
#[derive(Debug, Default)]
pub struct MemoryOracle {
    by_key: HashMap<String, DirAggregate>,
}

impl MemoryOracle {
    /// Index a scan for lookup.
    pub fn new(scan: &TreeScan) -> Self {
        Self {
            by_key: scan
                .dirs
                .iter()
                .map(|dir| (key_of(&dir.path), dir.clone()))
                .collect(),
        }
    }

    /// How many directories it knows about.
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Whether it knows about nothing.
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

impl SizeOracle for MemoryOracle {
    fn aggregate(&self, path: &Path) -> Option<DirAggregate> {
        self.by_key.get(&key_of(path)).cloned()
    }
}

/// Build the level for one directory.
///
/// Reads exactly one directory from disk — its immediate entries — and takes
/// every subdirectory's total from `oracle`. A subdirectory the oracle has never
/// heard of is reported with `sized: false` rather than a zero, because a zero
/// would be a claim and this is an absence.
pub fn children(path: &Path, root: &Path, oracle: &dyn SizeOracle) -> Result<TreeLevel> {
    let here = oracle.aggregate(path);
    let mut rows: Vec<TreeChild> = Vec::new();
    let mut readable = true;

    match std::fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let child = entry.path();
                let Ok(meta) = entry.metadata().or_else(|_| std::fs::symlink_metadata(&child))
                else {
                    continue;
                };
                let is_link = meta.file_type().is_symlink();
                let name = entry.file_name().to_string_lossy().into_owned();

                if is_link {
                    rows.push(TreeChild {
                        path: child,
                        name,
                        size_bytes: 0,
                        sized: true,
                        is_dir: meta.is_dir(),
                        is_link: true,
                        category: Category::Other,
                        item_count: 1,
                    });
                } else if meta.is_dir() {
                    let aggregate = oracle.aggregate(&child);
                    rows.push(TreeChild {
                        path: child,
                        name,
                        size_bytes: aggregate.as_ref().map_or(0, |a| a.total_bytes),
                        sized: aggregate.as_ref().is_some_and(DirAggregate::sized),
                        is_dir: true,
                        is_link: false,
                        category: Category::Directory,
                        item_count: 1,
                    });
                } else {
                    let category = Category::of(&child);
                    rows.push(TreeChild {
                        path: child,
                        name,
                        size_bytes: meta.len(),
                        sized: true,
                        is_dir: false,
                        is_link: false,
                        category,
                        item_count: 1,
                    });
                }
            }
        }
        Err(err) => {
            // Permission denied on a folder is ordinary on Windows and is not an
            // error the UI should show as a failure — it is a fact about the
            // folder, and the level says so.
            tracing::debug!(path = %path.display(), error = %err, "directory could not be listed");
            readable = false;
        }
    }

    rows.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.name.cmp(&b.name)));

    let mut folded = 0u64;
    if rows.len() > MAX_CHILDREN {
        let tail: Vec<TreeChild> = rows.split_off(MAX_CHILDREN);
        folded = tail.len() as u64;
        let bytes: u64 = tail.iter().map(|row| row.size_bytes).sum();
        // The tail is still drawn, as one rectangle, so the children continue to
        // add up to the parent. Dropping it would leave a hole the user reads as
        // free space.
        //
        // `sized` is the *conjunction* of the rows it stands for, not `true`.
        // Hardcoding `true` claimed the folded total was the whole truth even
        // when some of the folded rows were floors — which is reachable on any
        // cancelled scan, and `treemap_scan` deliberately caches and serves
        // those (security review M20). One unsized row in the tail makes the
        // tail unsized, exactly as `DirAggregate::sized` treats one unreadable
        // descendant.
        rows.push(TreeChild {
            path: PathBuf::new(),
            name: format!("… {folded} smaller items"),
            size_bytes: bytes,
            sized: tail.iter().all(|row| row.sized),
            is_dir: false,
            is_link: false,
            category: Category::Other,
            item_count: folded,
        });
    }

    Ok(TreeLevel {
        path: path.to_path_buf(),
        total_bytes: here.as_ref().map_or_else(
            || rows.iter().map(|row| row.size_bytes).sum(),
            |a| a.total_bytes,
        ),
        sized: here.as_ref().is_some_and(DirAggregate::sized),
        readable,
        children: rows,
        folded,
        unreadable_below: here.as_ref().map_or(0, |a| a.unreadable_below),
        compressed_below: here.as_ref().map_or(0, |a| a.compressed_below),
        breadcrumb: breadcrumb(path, root),
    })
}

/// Root-first list of ancestors from `root` down to `path`.
fn breadcrumb(path: &Path, root: &Path) -> Vec<PathBuf> {
    let mut crumbs = Vec::new();
    let mut current = Some(path);
    while let Some(here) = current {
        crumbs.push(here.to_path_buf());
        if crate::core::path_compare::same_path(here, root) {
            break;
        }
        current = here.parent();
    }
    crumbs.reverse();
    crumbs
}

/* -------------------------------------------------------------------------
 * The per-drive cache
 * ---------------------------------------------------------------------- */

/// The header row of a cached scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedScan {
    /// The drive root that was scanned.
    pub root: PathBuf,
    /// When.
    pub scanned_at: DateTime<Utc>,
    /// Total bytes found under it.
    pub total_bytes: u64,
    /// Files found.
    pub file_count: u64,
    /// Directories found.
    pub dir_count: u64,
    /// Directories that could not be listed.
    pub unreadable_dirs: u64,
    /// Whether the walk finished. A cancelled walk is stored so the partial
    /// picture is not thrown away, and flagged so nobody mistakes it for a total.
    pub complete: bool,
}

/// Store a scan, replacing whatever was cached for that root.
///
/// # Errors
/// Returns an error if the cache tables cannot be written.
pub fn save_cache(conn: &Connection, scan: &TreeScan) -> Result<()> {
    let root = key_of(&scan.root);
    // One transaction: a failure part-way rolls back and the previous cache
    // stays, instead of the root being left with no cache or half of one.
    let tx = conn.unchecked_transaction()?;
    let conn = &*tx;
    conn.execute("DELETE FROM treemap_dir WHERE drive = ?1", [&root])?;
    conn.execute("DELETE FROM treemap_scan WHERE drive = ?1", [&root])?;

    let total = scan.root_total();
    conn.execute(
        "INSERT INTO treemap_scan
            (drive, root_path, scanned_at, total_bytes, file_count, dir_count,
             unreadable_dirs, complete)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            &root,
            scan.root.to_string_lossy(),
            scan.scanned_at.to_rfc3339(),
            total.map_or(0, |t| t.total_bytes) as i64,
            total.map_or(0, |t| t.file_count) as i64,
            total.map_or(0, |t| t.dir_count) as i64,
            scan.unreadable.len() as i64,
            i64::from(!scan.cancelled),
        ],
    )?;

    let mut insert = conn.prepare(
        "INSERT INTO treemap_dir
            (drive, path, total_bytes, own_bytes, file_count, dir_count,
             readable, unreadable_below, compressed_below)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for dir in &scan.dirs {
        insert.execute(rusqlite::params![
            &root,
            key_of(&dir.path),
            dir.total_bytes as i64,
            dir.own_bytes as i64,
            dir.file_count as i64,
            dir.dir_count as i64,
            i64::from(dir.readable),
            dir.unreadable_below as i64,
            dir.compressed_below as i64,
        ])?;
    }
    drop(insert);
    tx.commit()?;
    Ok(())
}

/// The cached header for a root, when there is one.
///
/// # Errors
/// Returns an error if the cache table cannot be read.
pub fn cached_scan(conn: &Connection, root: &Path) -> Result<Option<CachedScan>> {
    let row = conn
        .query_row(
            "SELECT root_path, scanned_at, total_bytes, file_count, dir_count,
                    unreadable_dirs, complete
             FROM treemap_scan WHERE drive = ?1",
            [key_of(root)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()?;

    let Some((root_path, scanned_at, total, files, dirs, unreadable, complete)) = row else {
        return Ok(None);
    };
    let scanned_at = DateTime::parse_from_rfc3339(&scanned_at)
        .map_err(|err| CoreError::Quarantine(format!("unreadable cache timestamp: {err}")))?
        .with_timezone(&Utc);

    Ok(Some(CachedScan {
        root: PathBuf::from(root_path),
        scanned_at,
        total_bytes: total.max(0) as u64,
        file_count: files.max(0) as u64,
        dir_count: dirs.max(0) as u64,
        unreadable_dirs: unreadable.max(0) as u64,
        complete: complete != 0,
    }))
}

/// A [`SizeOracle`] backed by the SQLite cache.
pub struct CacheOracle<'a> {
    db: &'a dyn DbAccess,
    drive: String,
}

impl<'a> CacheOracle<'a> {
    /// Read totals for `root`'s cached scan.
    /// The connection is locked per lookup, so a long `children` walk never
    /// holds it for its whole run (#37).
    pub fn new(db: &'a dyn DbAccess, root: &Path) -> Self {
        Self {
            db,
            drive: key_of(root),
        }
    }
}

impl SizeOracle for CacheOracle<'_> {
    fn aggregate(&self, path: &Path) -> Option<DirAggregate> {
        with_db(self.db, |conn| {
            Ok(conn.query_row(
                "SELECT total_bytes, own_bytes, file_count, dir_count, readable,
                        unreadable_below, compressed_below
                 FROM treemap_dir WHERE drive = ?1 AND path = ?2",
                rusqlite::params![&self.drive, key_of(path)],
                |row| {
                    Ok(DirAggregate {
                        path: path.to_path_buf(),
                        total_bytes: row.get::<_, i64>(0)?.max(0) as u64,
                        own_bytes: row.get::<_, i64>(1)?.max(0) as u64,
                        file_count: row.get::<_, i64>(2)?.max(0) as u64,
                        dir_count: row.get::<_, i64>(3)?.max(0) as u64,
                        readable: row.get::<_, i64>(4)? != 0,
                        unreadable_below: row.get::<_, i64>(5)?.max(0) as u64,
                        compressed_below: row.get::<_, i64>(6)?.max(0) as u64,
                    })
                },
            )
            .optional()?)
        })
        .ok()
        .flatten()
    }
}

/* -------------------------------------------------------------------------
 * Tagging (C13, K-027)
 * ---------------------------------------------------------------------- */

/// The user picked the file by hand, so the opinion is theirs, not ours.
const TAGGED_CONFIDENCE: u8 = 100;

/// A tagged path the backend would not turn into an action, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaggedRefusal {
    /// The path as the UI sent it.
    pub path: PathBuf,
    /// Plain-language reason, shown verbatim.
    pub reason: String,
}

/// What [`propose`] made of a tagged list.
#[derive(Debug, Clone, Default)]
pub struct TaggedProposal {
    /// One per accepted file.
    pub findings: Vec<Finding>,
    /// One `Quarantine` recommendation per finding.
    pub recommendations: Vec<Recommendation>,
    /// Paths that were refused, in the order they arrived.
    pub refused: Vec<TaggedRefusal>,
}

/// Turn the paths the user tagged on the disk map into quarantine proposals.
///
/// The UI is not trusted: a protected path, anything that is not a plain file
/// (folders, links, missing paths) and duplicates are refused here, in code,
/// and reported back. Reads metadata only; nothing moves until `execute`.
pub fn propose(paths: &[PathBuf], brain: &Brain) -> TaggedProposal {
    let mut out = TaggedProposal::default();
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if !seen.insert(path.clone()) {
            continue;
        }
        match tagged_file(path, brain) {
            Ok((finding, recommendation)) => {
                out.findings.push(finding);
                out.recommendations.push(recommendation);
            }
            Err(reason) => out.refused.push(TaggedRefusal {
                path: path.clone(),
                reason,
            }),
        }
    }
    out
}

fn tagged_file(
    path: &Path,
    brain: &Brain,
) -> std::result::Result<(Finding, Recommendation), String> {
    if let Some(refusal) = brain.path_refusal(path) {
        return Err(refusal.reason);
    }
    if let Some(reason) = drive_root_refusal(path) {
        return Err(reason);
    }
    let meta = std::fs::symlink_metadata(path)
        .map_err(|err| format!("`{}` could not be read: {err}", path.display()))?;
    if !meta.file_type().is_file() {
        return Err(format!(
            "`{}` is not a plain file; only files can be tagged",
            path.display()
        ));
    }
    let modified = meta.modified().ok().map(DateTime::<Utc>::from);
    let mut finding = Finding::new(MODULE_ID, path, FindingKind::File);
    finding.size_bytes = meta.len();
    finding.modified = modified;
    let recommendation = Recommendation::new(
        vec![finding.id],
        RiskBucket::NeedsReview,
        TAGGED_CONFIDENCE,
        Action::Quarantine,
        "You tagged this file on the disk map.",
        meta.len(),
        Justification::UserTagged {
            size_bytes: meta.len(),
            modified,
        },
    )
    .map_err(|err| err.to_string())?;
    Ok((finding, recommendation))
}

/// Files sitting directly on a drive (or share) root, such as `C:\bootmgr`, are
/// never taggable: moving one can leave Windows unable to start, and then
/// WinKnife cannot run to restore it (K-028). Tagging-only rule; the global
/// protected list is unchanged. The resolved path is checked too, so `..` or a
/// junction cannot route around it.
fn drive_root_refusal(path: &Path) -> Option<String> {
    let resolved = std::fs::canonicalize(path).ok();
    let lexical = lexically_normal(path);
    let on_root = [path, lexical.as_path()]
        .into_iter()
        .chain(resolved.as_deref())
        .any(parent_is_root);
    on_root.then(|| {
        format!(
            "`{}` sits directly on the root of a drive, where Windows keeps its \
             start-up files; files there cannot be tagged",
            path.display()
        )
    })
}

/// Folds `.` and `..` without touching the disk; `canonicalize` fails when a
/// folder named before `..` does not exist, yet Windows still resolves the path.
fn lexically_normal(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::Prefix(_) | Component::RootDir)
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn parent_is_root(path: &Path) -> bool {
    use std::path::Component;
    path.parent().is_some_and(|parent| {
        let mut parts = parent.components().peekable();
        matches!(parts.peek(), Some(Component::Prefix(_)))
            && parts.all(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    })
}

/// Gate 4 for a tagged file: refuse when it is no longer the file the user saw.
///
/// The reason was "you tagged this file"; that stays true only while it is the
/// same plain file with the same size and modified time.
pub fn tagged_refusal(
    path: &Path,
    size_bytes: u64,
    modified: Option<DateTime<Utc>>,
) -> Option<String> {
    if let Some(reason) = drive_root_refusal(path) {
        return Some(format!("skipped: {reason}"));
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Some(format!(
            "skipped: `{}` is no longer there to move",
            path.display()
        ));
    };
    if !meta.file_type().is_file() {
        return Some(format!(
            "skipped: `{}` is no longer a plain file",
            path.display()
        ));
    }
    let now = meta.modified().ok().map(DateTime::<Utc>::from);
    if meta.len() != size_bytes || now != modified {
        return Some(format!(
            "skipped: `{}` changed after it was tagged, so it may no longer be \
             what you meant to remove",
            path.display()
        ));
    }
    None
}

/* -------------------------------------------------------------------------
 * Module registration
 * ---------------------------------------------------------------------- */

/// The treemap tool.
///
/// A treemap answers a question and has no opinions of its own: there is no
/// `analyze`. It executes only what the user tagged by hand ([`propose`]), and
/// only through the quarantine, so `has_execute` is true (K-027).
#[derive(Debug, Default)]
pub struct TreemapModule;

impl ToolModule for TreemapModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Disk map".to_owned(),
            description: "See where your disk space actually went, folder by folder.".to_owned(),
            icon: "treemap".to_owned(),
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
    use std::fs;

    /// A tree whose sizes are known exactly, so assertions are about the code
    /// and not about whatever the filesystem rounded to.
    ///
    /// ```text
    /// root/            2 000  (a.bin)
    ///   sub/           1 500  (b.bin 1 000 + deeper 500)
    ///     deeper/        500  (c.bin)
    ///   empty/           0
    /// ```
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("sub/deeper")).expect("dirs");
        fs::create_dir_all(dir.path().join("empty")).expect("dirs");
        fs::write(dir.path().join("a.bin"), vec![b'a'; 2_000]).expect("write");
        fs::write(dir.path().join("sub/b.bin"), vec![b'b'; 1_000]).expect("write");
        fs::write(dir.path().join("sub/deeper/c.bin"), vec![b'c'; 500]).expect("write");
        dir
    }

    fn scan_at(root: &Path) -> TreeScan {
        scan(root, &CancellationToken::new(), None)
    }

    fn aggregate_of<'a>(scan: &'a TreeScan, path: &Path) -> &'a DirAggregate {
        scan.dirs
            .iter()
            .find(|dir| crate::core::path_compare::same_path(&dir.path, path))
            .unwrap_or_else(|| panic!("no aggregate for {}", path.display()))
    }

    /* --- the folded tail row (security review M20) ---------------------- */

    /// An oracle that knows nothing, so every subdirectory comes back unsized.
    #[derive(Debug, Default)]
    struct BlindOracle;

    impl SizeOracle for BlindOracle {
        fn aggregate(&self, _path: &Path) -> Option<DirAggregate> {
            None
        }
    }

    #[test]
    fn a_folded_tail_of_unsized_rows_is_not_claimed_to_be_sized() {
        let dir = tempfile::tempdir().expect("tempdir");
        // More subdirectories than the fold limit, none of which the oracle has
        // an aggregate for. That is exactly the state a cancelled scan leaves,
        // and `treemap_scan` deliberately caches and serves those.
        for index in 0..(MAX_CHILDREN + 20) {
            fs::create_dir_all(dir.path().join(format!("d{index:04}"))).expect("dirs");
        }

        let level = children(dir.path(), dir.path(), &BlindOracle).expect("level");
        let tail = level.children.last().expect("a tail row");

        assert_eq!(level.folded, 20);
        assert_eq!(tail.item_count, 20);
        assert!(
            !tail.sized,
            "the tail stands for rows whose size is unknown; saying `sized` is the \
             silent zero this module rejects everywhere else"
        );
    }

    #[test]
    fn a_folded_tail_of_known_rows_is_still_sized() {
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0..(MAX_CHILDREN + 5) {
            fs::write(dir.path().join(format!("f{index:04}.bin")), b"x").expect("write");
        }

        let level = children(dir.path(), dir.path(), &BlindOracle).expect("level");
        let tail = level.children.last().expect("a tail row");
        assert_eq!(level.folded, 5);
        assert!(
            tail.sized,
            "files are measured directly, so a tail made only of files is a real total"
        );
    }

    /* --- the arena's keying (security review M21) ----------------------- */

    #[test]
    fn a_deep_chain_is_indexed_once_per_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut deep = dir.path().to_path_buf();
        for level in 0..40 {
            deep.push(format!("d{level:02}"));
        }
        fs::create_dir_all(&deep).expect("dirs");
        fs::write(deep.join("leaf.bin"), vec![b'z'; 700]).expect("write");

        let scan = scan_at(dir.path());

        assert_eq!(
            scan.dirs.len(),
            41,
            "one node per directory plus the root, with no spelling counted twice"
        );
        assert_eq!(scan.root_total().expect("root").total_bytes, 700);
        assert_eq!(scan.root_total().expect("root").dir_count, 40);
        assert_eq!(aggregate_of(&scan, &deep).own_bytes, 700);
    }

    /// Two spellings of one directory must still fold into one node — that is
    /// what the whole-path canonical key was there for, and the component key
    /// has to keep it.
    #[test]
    fn two_spellings_of_one_directory_are_one_node() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("Sub")).expect("dirs");

        let mut arena = DirArena::new(dir.path());
        let first = arena.ensure(&dir.path().join("Sub"));
        let second = arena.ensure(&dir.path().join("SUB"));
        let third = arena.ensure(&dir.path().join("sub."));

        assert_eq!(first, second, "case must not split a directory in two");
        assert_eq!(first, third, "nor must a trailing dot Win32 discards");
        assert_eq!(arena.dirs.len(), 2, "the root and one subdirectory");
    }

    #[test]
    fn the_arena_finds_an_ancestor_for_a_path_it_has_never_seen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut arena = DirArena::new(dir.path());
        let sub = arena.ensure(&dir.path().join("sub"));

        let buried = dir.path().join("sub").join("gone").join("deeper.txt");
        assert_eq!(arena.find(&buried), None);
        assert_eq!(arena.nearest_ancestor(&buried), Some(sub));
    }

    #[test]
    fn a_path_outside_the_root_gets_its_own_parentless_node() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("mapped");
        let mut arena = DirArena::new(&root);

        let outside = arena.ensure(&dir.path().join("elsewhere"));
        let again = arena.ensure(&dir.path().join("ELSEWHERE"));

        assert_eq!(outside, again, "still one node per place");
        assert_eq!(arena.parents[outside], None);
    }

    /// **The `du`-style oracle module spec 04's acceptance list asks for.**
    ///
    /// Sums the tree independently of the scan — a plain recursive walk with no
    /// arena, no parallelism and no aggregation pass — and demands the same
    /// number. If the bottom-up roll-up ever double-counts or drops a subtree,
    /// this is what says so.
    fn du(path: &Path) -> u64 {
        let mut total = 0;
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        for entry in entries.flatten() {
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            total += if meta.is_dir() {
                du(&entry.path())
            } else {
                meta.len()
            };
        }
        total
    }

    #[test]
    fn the_root_total_matches_a_du_style_oracle() {
        let dir = fixture();
        let scan = scan_at(dir.path());
        let root = scan.root_total().expect("a root aggregate");

        assert_eq!(root.total_bytes, du(dir.path()));
        assert_eq!(root.total_bytes, 3_500);
        assert_eq!(root.file_count, 3);
        // `sub`, `sub/deeper` and `empty`.
        assert_eq!(root.dir_count, 3);
    }

    #[test]
    fn every_directory_matches_the_oracle_at_its_own_level() {
        let dir = fixture();
        let scan = scan_at(dir.path());

        for aggregate in &scan.dirs {
            assert_eq!(
                aggregate.total_bytes,
                du(&aggregate.path),
                "`{}` disagrees with the oracle",
                aggregate.path.display()
            );
        }
        assert_eq!(aggregate_of(&scan, &dir.path().join("sub")).total_bytes, 1_500);
        assert_eq!(
            aggregate_of(&scan, &dir.path().join("sub/deeper")).total_bytes,
            500
        );
        assert_eq!(aggregate_of(&scan, &dir.path().join("empty")).total_bytes, 0);
    }

    #[test]
    fn own_bytes_counts_only_the_files_directly_inside() {
        let dir = fixture();
        let scan = scan_at(dir.path());
        let root = scan.root_total().expect("root");
        assert_eq!(root.own_bytes, 2_000, "a.bin only");
        assert_eq!(root.total_bytes, 3_500);
        assert_eq!(aggregate_of(&scan, &dir.path().join("sub")).own_bytes, 1_000);
    }

    #[test]
    fn an_empty_directory_is_still_a_node() {
        let dir = fixture();
        let scan = scan_at(dir.path());
        let empty = aggregate_of(&scan, &dir.path().join("empty"));
        assert_eq!(empty.total_bytes, 0);
        assert!(empty.sized(), "zero because it is empty, not because it is unknown");
    }

    #[test]
    fn a_cancelled_scan_says_so() {
        let dir = fixture();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let scan = scan(dir.path(), &cancel, None);
        assert!(scan.cancelled);
    }

    #[test]
    fn progress_is_reported_while_walking() {
        let dir = fixture();
        let mut seen = Vec::new();
        let mut sink = |p: &TreeProgress<'_>| seen.push(p.entries);
        scan(dir.path(), &CancellationToken::new(), Some(&mut sink));
        assert!(!seen.is_empty());
        assert!(seen.windows(2).all(|w| w[1] > w[0]), "counts must climb");
    }

    /// **Module spec 04: junctions are not followed and are counted once.**
    ///
    /// The junction points at a folder full of bytes that live elsewhere in the
    /// same tree. Counting through it would report those bytes twice, and the
    /// root would no longer match the oracle.
    #[cfg(windows)]
    #[test]
    fn a_junction_is_drawn_but_its_target_is_counted_once() {
        let dir = fixture();
        let door = dir.path().join("shortcut-to-sub");
        assert!(
            crate::core::test_support::make_junction(&door, &dir.path().join("sub")),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );

        let scan = scan_at(dir.path());
        let root = scan.root_total().expect("root");

        assert_eq!(
            root.total_bytes, 3_500,
            "the junction's target must not be counted a second time"
        );
        assert_eq!(root.total_bytes, du(dir.path()));
        assert!(
            scan.links.iter().any(|link| link == &door),
            "the junction must still be visible: {:?}",
            scan.links
        );
        // Compared as *text*, deliberately. `path_compare::same_path` resolves
        // both sides, and a junction resolves to its target — so it would report
        // the junction and `sub` as the same place, which is exactly true and
        // exactly not the question. What is being asserted here is that the
        // junction's own *name* never became a node (standing rule,
        // `docs/DECISIONS-NEEDED.md`: say why when a comparison stays lexical).
        assert!(
            !scan.dirs.iter().any(|d| d.path == door),
            "the junction's own name must not become a folder with contents of its own"
        );
    }

    #[test]
    fn children_serves_one_level_largest_first() {
        let dir = fixture();
        let scan = scan_at(dir.path());
        let oracle = MemoryOracle::new(&scan);

        let level = children(dir.path(), dir.path(), &oracle).expect("level");
        let names: Vec<_> = level.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a.bin", "sub", "empty"]);
        assert_eq!(level.children[0].size_bytes, 2_000);
        assert_eq!(level.children[1].size_bytes, 1_500, "aggregated, not own");
        assert_eq!(level.total_bytes, 3_500);
        assert!(level.sized);
        assert_eq!(level.breadcrumb, vec![dir.path().to_path_buf()]);
    }

    #[test]
    fn children_of_a_subdirectory_carry_a_root_first_breadcrumb() {
        let dir = fixture();
        let scan = scan_at(dir.path());
        let oracle = MemoryOracle::new(&scan);

        let level = children(&dir.path().join("sub/deeper"), dir.path(), &oracle).expect("level");
        assert_eq!(
            level.breadcrumb,
            vec![
                dir.path().to_path_buf(),
                dir.path().join("sub"),
                dir.path().join("sub/deeper"),
            ]
        );
    }

    /// A silent zero is a lie about where space went (module spec 04). A folder
    /// the oracle has never heard of is reported as unsized, and the UI has to
    /// render that differently from "empty".
    #[test]
    fn a_directory_the_scan_never_covered_is_unsized_rather_than_zero() {
        let dir = fixture();
        // An oracle that knows nothing at all.
        let oracle = MemoryOracle::default();

        let level = children(dir.path(), dir.path(), &oracle).expect("level");
        let sub = level
            .children
            .iter()
            .find(|c| c.name == "sub")
            .expect("sub is listed");
        assert_eq!(sub.size_bytes, 0);
        assert!(
            !sub.sized,
            "an unknown size must be marked unknown, never presented as empty"
        );
        assert!(
            level
                .children
                .iter()
                .find(|c| c.name == "a.bin")
                .expect("the file is listed")
                .sized,
            "a file's own size is always known"
        );
    }

    #[test]
    fn files_are_categorised_by_extension() {
        assert_eq!(Category::of(Path::new("a.MP4")), Category::Video);
        assert_eq!(Category::of(Path::new("a.jpeg")), Category::Image);
        assert_eq!(Category::of(Path::new("a.flac")), Category::Audio);
        assert_eq!(Category::of(Path::new("a.7z")), Category::Archive);
        assert_eq!(Category::of(Path::new("a.rs")), Category::Code);
        assert_eq!(Category::of(Path::new("a.dll")), Category::App);
        assert_eq!(Category::of(Path::new("a.pdf")), Category::Document);
        assert_eq!(Category::of(Path::new("a.qqq")), Category::Other);
        assert_eq!(Category::of(Path::new("noextension")), Category::Other);
    }

    #[test]
    fn a_long_directory_folds_its_tail_into_one_row_that_still_adds_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let count = MAX_CHILDREN + 50;
        let mut expected = 0u64;
        for index in 0..count {
            // Descending sizes, so the folded tail is the small end.
            let size = (count - index) as u64;
            fs::write(dir.path().join(format!("f{index:04}.bin")), vec![b'x'; size as usize])
                .expect("write");
            expected += size;
        }

        let scan = scan_at(dir.path());
        let oracle = MemoryOracle::new(&scan);
        let level = children(dir.path(), dir.path(), &oracle).expect("level");

        assert_eq!(level.children.len(), MAX_CHILDREN + 1, "the tail is one row");
        assert_eq!(level.folded, 50);
        let drawn: u64 = level.children.iter().map(|c| c.size_bytes).sum();
        assert_eq!(
            drawn, expected,
            "the rectangles must still add up to the folder, or the map shows a \
             hole the user reads as free space"
        );
        let tail = level.children.last().expect("tail row");
        assert_eq!(tail.item_count, 50);
        assert!(tail.name.contains("50"));
    }

    #[test]
    fn the_cache_round_trips_a_scan() {
        let dir = fixture();
        let conn = crate::core::db::open_in_memory().expect("db");
        let scan = scan_at(dir.path());

        save_cache(&conn, &scan).expect("save");
        let header = cached_scan(&conn, dir.path())
            .expect("read")
            .expect("a cached scan");

        assert_eq!(header.total_bytes, 3_500);
        assert_eq!(header.file_count, 3);
        assert!(header.complete);
        assert_eq!(header.root, dir.path());

        let oracle = CacheOracle::new(&conn, dir.path());
        assert_eq!(
            oracle
                .aggregate(&dir.path().join("sub"))
                .expect("cached sub")
                .total_bytes,
            1_500
        );
    }

    /// Audit A10: a save that fails part-way leaves the previous cache intact
    /// instead of an emptied one.
    #[test]
    fn a_failed_save_keeps_the_previous_cache() {
        let dir = fixture();
        let conn = crate::core::db::open_in_memory().expect("db");
        let good = scan_at(dir.path());
        save_cache(&conn, &good).expect("first");

        let mut broken = good.clone();
        let first = broken.dirs[0].clone();
        // The same primary key twice: the second insert fails, after the old
        // rows were deleted and one new one written.
        broken.dirs = vec![first.clone(), first];
        assert!(save_cache(&conn, &broken).is_err());

        let rows: i64 = conn
            .query_row("SELECT count(*) FROM treemap_dir", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows as usize, good.dirs.len(), "old rows must survive");
        assert!(cached_scan(&conn, dir.path()).expect("read").is_some());
    }

    #[test]
    fn saving_a_scan_twice_replaces_rather_than_duplicates() {
        let dir = fixture();
        let conn = crate::core::db::open_in_memory().expect("db");
        let scan = scan_at(dir.path());

        save_cache(&conn, &scan).expect("first");
        save_cache(&conn, &scan).expect("second");

        let rows: i64 = conn
            .query_row("SELECT count(*) FROM treemap_dir", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows as usize, scan.dirs.len());
        let headers: i64 = conn
            .query_row("SELECT count(*) FROM treemap_scan", [], |row| row.get(0))
            .expect("count");
        assert_eq!(headers, 1);
    }

    #[test]
    fn a_root_with_no_cache_reads_as_absent() {
        let conn = crate::core::db::open_in_memory().expect("db");
        assert_eq!(
            cached_scan(&conn, Path::new("Z:\\")).expect("read"),
            None,
            "never scanned is not an error"
        );
    }

    #[test]
    fn a_cached_level_is_served_without_re_walking_the_tree() {
        let dir = fixture();
        let conn = crate::core::db::open_in_memory().expect("db");
        save_cache(&conn, &scan_at(dir.path())).expect("save");

        let level = children(dir.path(), dir.path(), &CacheOracle::new(&conn, dir.path()))
            .expect("level");
        assert_eq!(level.total_bytes, 3_500);
        assert_eq!(
            level
                .children
                .iter()
                .find(|c| c.name == "sub")
                .expect("sub")
                .size_bytes,
            1_500
        );
    }

    #[test]
    fn the_manifest_declares_scan_and_tagged_execute() {
        let manifest = TreemapModule.manifest();
        assert_eq!(manifest.id, "treemap");
        assert!(manifest.capabilities.has_scan);
        assert!(manifest.capabilities.has_execute, "tagged files can be quarantined");
    }

    #[test]
    fn propose_accepts_files_and_refuses_folders_protected_and_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("big.bin");
        fs::write(&file, vec![0u8; 1_234]).expect("write");
        let folder = dir.path().join("folder");
        fs::create_dir(&folder).expect("mkdir");
        let missing = dir.path().join("gone.bin");
        let guarded = dir.path().join("guarded");
        fs::create_dir(&guarded).expect("mkdir");
        let protected = guarded.join("system.dll");
        fs::write(&protected, b"x").expect("write");

        let brain = Brain::builder()
            .protected_paths(crate::core::brain::ProtectedPaths::from_trees([
                fs::canonicalize(&guarded).expect("canonical"),
            ]))
            .build();
        let out = propose(
            &[
                file.clone(),
                folder.clone(),
                protected.clone(),
                missing.clone(),
                file.clone(),
            ],
            &brain,
        );

        assert_eq!(out.findings.len(), 1, "the duplicate entry is dropped");
        assert_eq!(out.findings[0].path, file);
        assert_eq!(out.findings[0].size_bytes, 1_234);
        let rec = &out.recommendations[0];
        assert_eq!(rec.action, Action::Quarantine);
        assert_eq!(rec.risk, RiskBucket::NeedsReview);
        assert!(!rec.reason.is_empty());
        assert!(matches!(
            rec.justification,
            Justification::UserTagged { size_bytes: 1_234, .. }
        ));

        let refused: Vec<_> = out.refused.iter().map(|r| r.path.clone()).collect();
        assert_eq!(refused, vec![folder, protected, missing]);
        assert!(out.refused.iter().all(|r| !r.reason.is_empty()));
        assert!(out.refused[0].reason.contains("only files"));
    }

    #[test]
    fn tagged_refusal_passes_an_unchanged_file_and_refuses_a_changed_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.bin");
        fs::write(&file, b"abc").expect("write");
        let out = propose(std::slice::from_ref(&file), &Brain::new());
        let Justification::UserTagged {
            size_bytes,
            modified,
        } = out.recommendations[0].justification.clone()
        else {
            panic!("expected a UserTagged justification");
        };
        assert_eq!(tagged_refusal(&file, size_bytes, modified), None);

        fs::write(&file, b"abcdef").expect("rewrite");
        let reason = tagged_refusal(&file, size_bytes, modified).expect("refused");
        assert!(reason.contains("changed after it was tagged"));
    }

    #[test]
    fn a_file_on_a_drive_root_is_refused_and_one_level_down_is_not() {
        for root_file in [r"C:\bootmgr", r"\\?\C:\BOOTNXT", r"C:\x\..\bootmgr",
            // `canonicalize` fails here, so the lexical fold must catch it.
            r"C:\winknife-no-such-dir\.\..\bootmgr",
        ] {
            let path = Path::new(root_file);
            let reason = drive_root_refusal(path).expect(root_file);
            assert!(reason.contains("root of a drive"), "{reason}");
            assert!(tagged_refusal(path, 0, None)
                .expect("gate 4 refuses")
                .contains("root of a drive"));
            let out = propose(&[path.to_path_buf()], &Brain::new());
            // The global protected check may refuse first, with its own reason.
            assert!(out.findings.is_empty(), "{root_file} must not be proposed");
            assert_eq!(out.refused.len(), 1);
        }
        assert_eq!(drive_root_refusal(Path::new(r"C:\Data\bootmgr")), None);

        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("one-level-down.bin");
        fs::write(&file, b"x").expect("write");
        let out = propose(std::slice::from_ref(&file), &Brain::new());
        assert_eq!(out.findings.len(), 1, "{:?}", out.refused);
    }

    #[test]
    fn tagged_refusal_refuses_a_folder_and_a_missing_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(tagged_refusal(dir.path(), 0, None)
            .expect("folder refused")
            .contains("no longer a plain file"));
        assert!(tagged_refusal(&dir.path().join("x"), 0, None).is_some());
    }

    /// `analyze` has no opinions to record, so it must produce none — and in
    /// particular must never produce an actionable recommendation, which since
    /// phase 3 would need a justification it has no way to supply.
    #[test]
    fn the_treemap_proposes_nothing() {
        let brain = crate::core::brain::Brain::new();
        assert!(TreemapModule.analyze(&[], &brain).is_empty());
    }
}
