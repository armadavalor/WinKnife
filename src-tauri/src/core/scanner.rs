//! The shared filesystem walker every scanning module uses.
//!
//! Why shared: the cleaner, the treemap and the duplicate finder all want "walk
//! this tree, tell me about the files, let me cancel, do not follow links".
//! Writing that three times is how three subtly different bugs appear.
//!
//! Read-only by construction — the walker has no way to modify anything, which
//! is what makes `scan()` honest about being read-only (CLAUDE.md hard rules).
//!
//! # Why `read_dir` and not a parallel crate
//!
//! This used to be `jwalk`, which walks directories on a rayon pool, and it was
//! **sixty times slower** than what is here now. The cost was never the
//! traversal: it was that `jwalk::DirEntry::metadata()` calls
//! `fs::symlink_metadata`, which on Windows opens the file. One open per entry.
//!
//! Measured on the phase-3 100 000-file synthetic tree
//! (`tests/treemap_perf.rs`), release build, warm cache:
//!
//! ```text
//! jwalk enumeration alone                      64 ms
//! jwalk + fs::symlink_metadata per entry     3924 ms
//! read_dir + DirEntry::metadata()             143 ms
//! ```
//!
//! `std::fs::DirEntry::metadata()` answers from the `WIN32_FIND_DATA` the
//! directory listing already returned, and does not follow reparse points —
//! which is exactly the semantics this walker wants. So the parallelism was
//! worth 80 ms and the syscall was worth 3 800 ms, and the walk is now a plain
//! sorted depth-first `read_dir`: streaming, deterministic and no arena.
//!
//! **With one caveat, and it is why [`WalkOptions::accurate_timestamps`]
//! exists.** NTFS keeps a *copy* of each entry's timestamps in its parent's
//! index and updates it lazily, so the listing can report a modification time
//! that is older than the truth. Sizes and attributes are not affected. The
//! cleaner's age heuristic reads modification times, and "older than the truth"
//! is the direction that makes a freshly written file look abandoned — so the
//! cleaner and the duplicate finder keep the authoritative stat and are exactly
//! as fast as they were. The treemap opts out, because it reads no clocks at
//! all, and it is the only thing here that walks a whole drive
//! (`docs/DECISIONS-NEEDED.md`, phase 3).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};

/// Cooperative cancellation, checked once per directory entry.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// A token that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the walk to stop. Idempotent.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// One file the walker saw. Directories are reported too so a module can decide
/// what to do with them; the cleaner only takes files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// Absolute path.
    pub path: PathBuf,
    /// Size in bytes (`0` for directories — sizing a tree is the caller's job).
    pub size_bytes: u64,
    /// Last modification time, when the filesystem exposes one.
    pub modified: Option<DateTime<Utc>>,
    /// Creation time, when available.
    pub created: Option<DateTime<Utc>>,
    /// Last access time. Often disabled system-wide on Windows.
    pub accessed: Option<DateTime<Utc>>,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Whether this entry is a reparse point (a symlink or a junction).
    ///
    /// Only ever `true` when the caller asked for links with
    /// [`WalkOptions::report_links`]; the walk never descends into one either
    /// way.
    pub is_link: bool,
    /// Whether NTFS stores this file compressed or sparse, so `size_bytes` is
    /// larger than the room it actually takes up.
    pub compressed: bool,
}

/// How a walk should behave.
#[derive(Debug, Clone, Copy)]
pub struct WalkOptions {
    /// Report reparse points as entries instead of passing over them.
    ///
    /// Off by default, which is what every module before the treemap wanted: a
    /// cleaner that saw a junction could offer to quarantine it, and moving a
    /// link moves the link rather than the data.
    ///
    /// The treemap wants them *visible* precisely because it must not count them:
    /// a junction pointing at a 40 GB folder would otherwise make that folder's
    /// bytes appear twice, and simply hiding it leaves a hole in a picture whose
    /// whole job is to add up to the disk (module spec 04). Reported entries
    /// carry `size_bytes: 0` and `is_link: true`, and the walk still refuses to
    /// descend into them.
    pub report_links: bool,

    /// Whether timestamps have to be authoritative, at one extra syscall per
    /// entry.
    ///
    /// **On by default, and every module before the treemap needs it on.**
    ///
    /// The directory listing Windows returns carries a *copy* of each entry's
    /// timestamps, and NTFS updates that copy lazily — a file or folder that
    /// changed moments ago can still be listed with its previous modification
    /// time. Sizes and attributes do not have this problem, which is why the
    /// cheap path is still the default for everything else.
    ///
    /// The cleaner's age heuristic reads modification times, and a stale one
    /// always reads *older* than the truth. That is the dangerous direction:
    /// a file a program has just written would look untouched for days. So the
    /// cleaner and the duplicate finder pay the syscall and get the real
    /// answer, exactly as they did before this walker was rewritten.
    ///
    /// The treemap turns it off because it never looks at a timestamp — it
    /// wants sizes — and it is the only thing here that walks a whole drive.
    pub accurate_timestamps: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            report_links: false,
            accurate_timestamps: true,
        }
    }
}

impl WalkEntry {
    /// Age in hours since the last modification, using `now` as the reference.
    /// A file with no modification time counts as brand new, so age filters
    /// leave it alone.
    pub fn age_hours(&self, now: DateTime<Utc>) -> i64 {
        self.modified
            .map(|modified| (now - modified).num_hours())
            .unwrap_or(0)
    }
}

/// How a walk behaved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WalkSummary {
    /// Entries handed to the callback.
    pub entries_seen: u64,
    /// Directories the walker could not read (permissions, transient locks).
    pub unreadable: Vec<PathBuf>,
    /// Whether the walk stopped early because of cancellation.
    pub cancelled: bool,
}

/// Walk `root`, calling `visit` for every entry.
///
/// Symbolic links and junctions are never followed: a reparse point can send a
/// walk out of the tree the user agreed to scan, and the whole promise of this
/// app is that it stays where it was invited.
///
/// Returns a summary rather than a `Result` — an unreadable subdirectory is
/// normal on Windows and must not abort a scan.
pub fn walk(
    root: &Path,
    cancel: &CancellationToken,
    visit: &mut dyn FnMut(&WalkEntry),
) -> WalkSummary {
    walk_with(root, WalkOptions::default(), cancel, visit)
}

/// [`walk`] with the behaviour switches in [`WalkOptions`].
pub fn walk_with(
    root: &Path,
    options: WalkOptions,
    cancel: &CancellationToken,
    visit: &mut dyn FnMut(&WalkEntry),
) -> WalkSummary {
    let mut summary = WalkSummary::default();

    if !root.exists() {
        // A rule pointing at a folder this machine does not have is not an
        // error; it just contributes nothing.
        return summary;
    }

    // Depth-first, deepest-last, so the emission order is the sorted DFS order
    // every caller before this one relied on.
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }

        let listing = match std::fs::read_dir(&dir) {
            Ok(listing) => listing,
            Err(_) => {
                // Permission denied, or it went away between being listed and
                // being opened. Both are ordinary on Windows and neither may
                // abort a scan.
                summary.unreadable.push(dir);
                continue;
            }
        };

        let mut entries: Vec<WalkEntry> = Vec::new();
        for entry in listing.flatten() {
            // `DirEntry::metadata` answers from the directory listing Windows
            // already returned, and does not follow a reparse point. That is the
            // whole reason this walker exists in this shape — see the module
            // header for the measurement.
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let is_link = metadata.file_type().is_symlink();
            if is_link && !options.report_links {
                continue;
            }
            let is_dir = metadata.is_dir();
            let path = entry.path();
            // The listing's copy of the timestamps can lag behind the truth;
            // the size and the attributes cannot. A caller that reads clocks
            // pays for the authoritative answer — see `accurate_timestamps`.
            let clocks = if options.accurate_timestamps {
                std::fs::symlink_metadata(&path).unwrap_or_else(|_| metadata.clone())
            } else {
                metadata.clone()
            };
            entries.push(WalkEntry {
                path,
                // A link's own size is meaningless and its target's bytes belong
                // to wherever the target really lives, so it contributes nothing.
                size_bytes: if is_dir || is_link { 0 } else { metadata.len() },
                modified: to_utc(clocks.modified().ok()),
                created: to_utc(clocks.created().ok()),
                accessed: to_utc(clocks.accessed().ok()),
                is_dir,
                is_link,
                compressed: is_compressed(&metadata),
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        // Push children in reverse so the stack pops them in sorted order.
        let mut descend: Vec<PathBuf> = Vec::new();
        for entry in &entries {
            if cancel.is_cancelled() {
                summary.cancelled = true;
                break;
            }
            summary.entries_seen += 1;
            visit(entry);
            // Never through a reparse point: it can send a walk out of the tree
            // the user agreed to scan, and the whole promise of this app is that
            // it stays where it was invited.
            if entry.is_dir && !entry.is_link {
                descend.push(entry.path.clone());
            }
        }
        if summary.cancelled {
            break;
        }
        descend.reverse();
        stack.extend(descend);
    }

    summary
}

/// Whether the directory listing says this file is compressed or sparse.
///
/// Read from metadata the walk already has, never from a second syscall — the
/// treemap asks this of every file on a drive.
#[cfg(windows)]
fn is_compressed(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x0000_0800;
    const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x0000_0200;
    metadata.file_attributes() & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE) != 0
}

#[cfg(not(windows))]
fn is_compressed(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn to_utc(time: Option<std::time::SystemTime>) -> Option<DateTime<Utc>> {
    time.map(DateTime::<Utc>::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("sub/deeper")).expect("dirs");
        fs::write(dir.path().join("a.log"), b"aaaa").expect("write");
        fs::write(dir.path().join("sub/b.tmp"), b"bb").expect("write");
        fs::write(dir.path().join("sub/deeper/c.bin"), b"c").expect("write");
        dir
    }

    fn collect(root: &Path, cancel: &CancellationToken) -> (Vec<WalkEntry>, WalkSummary) {
        let mut entries = Vec::new();
        let summary = walk(root, cancel, &mut |entry| entries.push(entry.clone()));
        (entries, summary)
    }

    #[test]
    fn walking_finds_every_file_at_every_depth() {
        let dir = fixture();
        let (entries, summary) = collect(dir.path(), &CancellationToken::new());

        let files: Vec<_> = entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| {
                e.path
                    .file_name()
                    .expect("name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();

        assert_eq!(files, vec!["a.log", "b.tmp", "c.bin"]);
        assert!(!summary.cancelled);
        assert!(summary.unreadable.is_empty());
        assert_eq!(summary.entries_seen, entries.len() as u64);
    }

    #[test]
    fn file_sizes_are_reported_and_directories_are_zero() {
        let dir = fixture();
        let (entries, _) = collect(dir.path(), &CancellationToken::new());

        let by_name = |name: &str| {
            entries
                .iter()
                .find(|e| e.path.file_name().is_some_and(|n| n == name))
                .expect("entry")
        };

        assert_eq!(by_name("a.log").size_bytes, 4);
        assert_eq!(by_name("b.tmp").size_bytes, 2);
        assert_eq!(by_name("sub").size_bytes, 0);
        assert!(by_name("sub").is_dir);
    }

    #[test]
    fn timestamps_come_through() {
        let dir = fixture();
        let (entries, _) = collect(dir.path(), &CancellationToken::new());
        let file = entries.iter().find(|e| !e.is_dir).expect("a file");
        assert!(file.modified.is_some(), "modification time is required");
        assert!(file.age_hours(Utc::now()) < 1);
    }

    #[test]
    fn age_is_measured_from_the_modification_time() {
        let entry = WalkEntry {
            path: PathBuf::from("C:\\x"),
            size_bytes: 0,
            modified: Some(Utc::now() - chrono::Duration::hours(30)),
            created: None,
            accessed: None,
            is_dir: false,
            is_link: false,
            compressed: false,
        };
        assert_eq!(entry.age_hours(Utc::now()), 30);
    }

    #[test]
    fn an_entry_without_a_modification_time_counts_as_new() {
        let entry = WalkEntry {
            path: PathBuf::from("C:\\x"),
            size_bytes: 0,
            modified: None,
            created: None,
            accessed: None,
            is_dir: false,
            is_link: false,
            compressed: false,
        };
        assert_eq!(entry.age_hours(Utc::now()), 0);
    }

    #[test]
    fn a_missing_root_yields_an_empty_walk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (entries, summary) = collect(&dir.path().join("nope"), &CancellationToken::new());
        assert!(entries.is_empty());
        assert_eq!(summary, WalkSummary::default());
    }

    #[test]
    fn cancellation_stops_the_walk() {
        let dir = fixture();
        let cancel = CancellationToken::new();
        cancel.cancel();

        let (entries, summary) = collect(dir.path(), &cancel);

        assert!(entries.is_empty());
        assert!(summary.cancelled);
    }

    #[test]
    fn a_token_starts_uncancelled_and_is_idempotent() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn symlinks_are_never_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = dir.path().join("outside");
        let inside = dir.path().join("inside");
        fs::create_dir_all(&outside).expect("dirs");
        fs::create_dir_all(&inside).expect("dirs");
        fs::write(outside.join("secret.txt"), b"do not walk here").expect("write");

        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, inside.join("link")).is_ok();
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&outside, inside.join("link")).is_ok();

        if !made {
            // M11: loud, not silent. This used to `return` and pass.
            crate::core::test_support::privileged_operation_unavailable(
                "symlinks_are_never_followed",
                "creating a directory symlink needs Developer Mode or elevation",
            );
            return;
        }

        let (entries, _) = collect(&inside, &CancellationToken::new());
        assert!(
            !entries.iter().any(|e| e.path.ends_with("secret.txt")),
            "the walker followed a link out of its tree"
        );
    }

    /// The same guarantee through a **junction**, which any standard user can
    /// create — so unlike the symlink test above, this one asserts on every
    /// developer machine rather than only in CI
    /// (`docs/DECISIONS-NEEDED.md`, architect resolution before phase 2).
    #[cfg(windows)]
    #[test]
    fn junctions_are_never_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = dir.path().join("outside");
        let inside = dir.path().join("inside");
        fs::create_dir_all(&outside).expect("dirs");
        fs::create_dir_all(&inside).expect("dirs");
        fs::write(outside.join("secret.txt"), b"do not walk here").expect("write");

        let door = inside.join("door");
        assert!(
            crate::core::test_support::make_junction(&door, &outside),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );
        assert!(
            fs::symlink_metadata(&door)
                .expect("the junction exists")
                .is_symlink(),
            "the premise: Windows reports a junction as a link"
        );

        let (entries, _) = collect(&inside, &CancellationToken::new());
        assert!(
            !entries.iter().any(|e| e.path.ends_with("secret.txt")),
            "the walker followed a junction out of its tree"
        );
    }

    /// The privilege-free companion to the test above: whatever the tree
    /// contains, every entry a walk reports is inside the root it was given.
    /// That is the invariant a followed link breaks, and it is asserted on
    /// every machine, privileged or not (security review M11).
    #[test]
    fn a_walk_never_reports_anything_outside_its_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = dir.path().join("outside");
        let inside = dir.path().join("inside");
        fs::create_dir_all(outside.join("deep")).expect("dirs");
        fs::create_dir_all(inside.join("sub/deeper")).expect("dirs");
        fs::write(outside.join("deep/secret.txt"), b"do not walk here").expect("write");
        fs::write(inside.join("sub/deeper/ours.txt"), b"fine").expect("write");

        let root = dunce::canonicalize(&inside).expect("canonicalize");
        let (entries, summary) = collect(&inside, &CancellationToken::new());

        assert!(entries.iter().any(|e| e.path.ends_with("ours.txt")));
        assert!(summary.entries_seen > 0);
        for entry in &entries {
            let seen = dunce::canonicalize(&entry.path).expect("canonicalize entry");
            assert!(
                seen.starts_with(&root),
                "`{}` is outside the walk root `{}`",
                seen.display(),
                root.display()
            );
        }
    }

    /// The treemap needs to *see* a junction so it can draw it without counting
    /// it. Opting in reports it; the walk still refuses to go through it.
    #[cfg(windows)]
    #[test]
    fn an_opted_in_walk_reports_a_junction_without_descending_into_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = dir.path().join("outside");
        let inside = dir.path().join("inside");
        fs::create_dir_all(&outside).expect("dirs");
        fs::create_dir_all(&inside).expect("dirs");
        fs::write(outside.join("secret.txt"), vec![b'x'; 4096]).expect("write");

        let door = inside.join("door");
        assert!(
            crate::core::test_support::make_junction(&door, &outside),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );

        let mut entries = Vec::new();
        walk_with(
            &inside,
            WalkOptions {
                report_links: true,
                ..WalkOptions::default()
            },
            &CancellationToken::new(),
            &mut |entry| entries.push(entry.clone()),
        );

        let link = entries
            .iter()
            .find(|e| e.path == door)
            .expect("the junction must be reported when asked for");
        assert!(link.is_link);
        assert_eq!(
            link.size_bytes, 0,
            "a link contributes no bytes; its target's bytes belong to the target"
        );
        assert!(
            !entries.iter().any(|e| e.path.ends_with("secret.txt")),
            "reporting a link must not mean walking through it"
        );
    }

    /// The reason `accurate_timestamps` defaults to on.
    ///
    /// The directory listing's copy of a timestamp can lag behind the real one,
    /// and the cleaner decides what is old enough to remove from exactly that
    /// number. Two walks of an untouched tree must agree, or the age heuristic
    /// is reading noise.
    #[test]
    fn two_walks_of_an_untouched_tree_report_the_same_clocks() {
        let dir = fixture();
        let clocks = |root: &Path| {
            let mut seen: Vec<(PathBuf, Option<DateTime<Utc>>)> = Vec::new();
            walk(root, &CancellationToken::new(), &mut |entry| {
                seen.push((entry.path.clone(), entry.modified));
            });
            seen.sort();
            seen
        };
        assert_eq!(
            clocks(dir.path()),
            clocks(dir.path()),
            "a walk that reports authoritative clocks must be repeatable"
        );
    }

    #[test]
    fn the_default_options_ask_for_authoritative_clocks() {
        let options = WalkOptions::default();
        assert!(
            options.accurate_timestamps,
            "every caller but the treemap reads modification times, and a stale              one reads older than the truth"
        );
        assert!(!options.report_links);
    }

    #[test]
    fn the_walk_order_is_stable_across_runs() {
        let dir = fixture();
        let paths =
            |entries: Vec<WalkEntry>| entries.into_iter().map(|e| e.path).collect::<Vec<_>>();
        let (first, _) = collect(dir.path(), &CancellationToken::new());
        let (second, _) = collect(dir.path(), &CancellationToken::new());

        // Paths and their order, not the whole entry: reading a file can move
        // its access time, and the fixture assertions elsewhere already pin the
        // sizes. What a deterministic scan means is "the same items, in the
        // same order".
        assert_eq!(paths(first), paths(second), "scans must be deterministic");
    }
}
