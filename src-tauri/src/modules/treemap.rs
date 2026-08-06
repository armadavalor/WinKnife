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

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::path_compare::CanonicalPath;
use crate::core::scanner::{self, CancellationToken, WalkOptions};

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
    // `index[key]` is the position of a directory in `dirs`. Keyed through
    // `CanonicalPath` rather than by text: a walk can hand back two spellings of
    // one directory (an 8.3 alias in the root, a long name below it) and two
    // spellings would otherwise become two folders whose sizes each miss half
    // the files (standing rule, `docs/DECISIONS-NEEDED.md`).
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut dirs: Vec<DirAggregate> = Vec::new();
    let mut parents: Vec<Option<usize>> = Vec::new();
    let mut depths: Vec<u32> = Vec::new();
    let mut links = Vec::new();
    let mut last_parent: Option<(PathBuf, usize)> = None;

    let mut seen = 0u64;
    let summary = scanner::walk_with(
        root,
        WalkOptions {
            report_links: true,
            // The treemap reads no clocks: every figure it shows is a size.
            // Skipping the authoritative stat is what makes a whole-drive walk
            // affordable (see `core::scanner`'s header for the measurement).
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
                ensure_dir(
                    &entry.path,
                    root,
                    &mut index,
                    &mut dirs,
                    &mut parents,
                    &mut depths,
                );
                return;
            }

            let Some(parent) = entry.path.parent() else {
                return;
            };
            // The walk is depth-first and sorted, so a run of files shares one
            // parent. Remembering the last one turns a canonical-key computation
            // and a hash lookup *per file* into one per directory, which on a
            // hundred thousand files is a hundred thousand allocations saved.
            let slot = match &last_parent {
                Some((path, slot)) if path == parent => *slot,
                _ => {
                    let slot =
                        ensure_dir(parent, root, &mut index, &mut dirs, &mut parents, &mut depths);
                    last_parent = Some((parent.to_path_buf(), slot));
                    slot
                }
            };
            let node = &mut dirs[slot];
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

    // A directory the walk could not list is marked rather than silently sized
    // at zero. `unreadable` names the directory itself, which is usually already
    // a node (its parent listed it) — if it is not, its nearest known ancestor
    // still learns that something below it is missing.
    for missing in &summary.unreadable {
        match index.get(&key_of(missing)) {
            Some(slot) => dirs[*slot].readable = false,
            None => {
                if let Some(slot) = nearest_ancestor(missing, &index) {
                    dirs[slot].unreadable_below += 1;
                }
            }
        }
    }

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

/// The comparison key for a directory path.
fn key_of(path: &Path) -> String {
    // `lexical` rather than `resolved`: this runs once per directory entry on a
    // walk of a whole drive, and resolving would be a syscall each time. The
    // walk hands back paths it built itself by joining onto one root, so they
    // are already one consistent spelling; the canonicalisation that matters
    // (case, separators, trailing dots) is what `lexical` does.
    CanonicalPath::lexical(path).key().to_owned()
}

/// Find or create the node for `path`, wiring it to its parent.
fn ensure_dir(
    path: &Path,
    root: &Path,
    index: &mut HashMap<String, usize>,
    dirs: &mut Vec<DirAggregate>,
    parents: &mut Vec<Option<usize>>,
    depths: &mut Vec<u32>,
) -> usize {
    let key = key_of(path);
    if let Some(slot) = index.get(&key) {
        return *slot;
    }

    // The root has no parent inside the tree; everything else finds or creates
    // one, which also fills in a directory the walk never listed directly.
    let parent = if crate::core::path_compare::same_path(path, root) {
        None
    } else {
        path.parent()
            .filter(|parent| crate::core::path_compare::is_inside(root, parent))
            .map(|parent| ensure_dir(parent, root, index, dirs, parents, depths))
    };

    let slot = dirs.len();
    dirs.push(DirAggregate {
        path: path.to_path_buf(),
        total_bytes: 0,
        own_bytes: 0,
        file_count: 0,
        dir_count: 0,
        readable: true,
        unreadable_below: 0,
        compressed_below: 0,
    });
    depths.push(parent.map_or(0, |p| depths[p] + 1));
    parents.push(parent);
    index.insert(key, slot);
    slot
}

/// The deepest known directory that contains `path`.
fn nearest_ancestor(path: &Path, index: &HashMap<String, usize>) -> Option<usize> {
    let mut current = path.parent();
    while let Some(dir) = current {
        if let Some(slot) = index.get(&key_of(dir)) {
            return Some(*slot);
        }
        current = dir.parent();
    }
    None
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
        rows.push(TreeChild {
            path: PathBuf::new(),
            name: format!("… {folded} smaller items"),
            size_bytes: bytes,
            sized: true,
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
    conn: &'a Connection,
    drive: String,
}

impl<'a> CacheOracle<'a> {
    /// Read totals for `root`'s cached scan.
    pub fn new(conn: &'a Connection, root: &Path) -> Self {
        Self {
            conn,
            drive: key_of(root),
        }
    }
}

impl SizeOracle for CacheOracle<'_> {
    fn aggregate(&self, path: &Path) -> Option<DirAggregate> {
        self.conn
            .query_row(
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
            .optional()
            .ok()
            .flatten()
    }
}

/* -------------------------------------------------------------------------
 * Module registration
 * ---------------------------------------------------------------------- */

/// The treemap tool.
///
/// It scans but never executes: a treemap answers a question, and the actions it
/// leads to are the cleaner's and the duplicate finder's. `has_execute` is
/// therefore false, and there is no `analyze` — every rectangle is a fact, and
/// this module has no opinions to record.
#[derive(Debug, Default)]
pub struct TreemapModule;

impl ToolModule for TreemapModule {
    fn id(&self) -> &'static str {
        "treemap"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Disk map".to_owned(),
            description: "See where your disk space actually went, folder by folder.".to_owned(),
            icon: "treemap".to_owned(),
            capabilities: ModuleCapabilities {
                has_scan: true,
                has_execute: false,
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
    fn the_manifest_declares_a_scan_only_module() {
        let manifest = TreemapModule.manifest();
        assert_eq!(manifest.id, "treemap");
        assert!(manifest.capabilities.has_scan);
        assert!(
            !manifest.capabilities.has_execute,
            "a map answers a question; the cleaner and the duplicate finder act"
        );
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
