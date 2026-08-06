//! The shared filesystem walker every scanning module uses.
//!
//! Why shared: the cleaner, the treemap and the duplicate finder all want "walk
//! this tree, tell me about the files, let me cancel, do not follow links".
//! Writing that three times is how three subtly different bugs appear.
//!
//! Read-only by construction — the walker has no way to modify anything, which
//! is what makes `scan()` honest about being read-only (CLAUDE.md hard rules).

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
    let mut summary = WalkSummary::default();

    if !root.exists() {
        // A rule pointing at a folder this machine does not have is not an
        // error; it just contributes nothing.
        return summary;
    }

    let walker = jwalk::WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .sort(true);

    for entry in walker {
        if cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                if let Some(path) = err.path() {
                    summary.unreadable.push(path.to_path_buf());
                }
                continue;
            }
        };

        // `file_type` comes from the directory listing and does not follow the
        // link, which is what we want.
        if entry.path_is_symlink() {
            continue;
        }

        let is_dir = entry.file_type().is_dir();
        let metadata = entry.metadata().ok();
        let walk_entry = WalkEntry {
            path: entry.path(),
            size_bytes: metadata.as_ref().filter(|_| !is_dir).map_or(0, |m| m.len()),
            modified: metadata.as_ref().and_then(|m| to_utc(m.modified().ok())),
            created: metadata.as_ref().and_then(|m| to_utc(m.created().ok())),
            accessed: metadata.as_ref().and_then(|m| to_utc(m.accessed().ok())),
            is_dir,
        };

        summary.entries_seen += 1;
        visit(&walk_entry);
    }

    summary
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
            .map(|e| e.path.file_name().expect("name").to_string_lossy().into_owned())
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
            // Symlink creation needs developer mode or elevation on Windows.
            return;
        }

        let (entries, _) = collect(&inside, &CancellationToken::new());
        assert!(
            !entries.iter().any(|e| e.path.ends_with("secret.txt")),
            "the walker followed a link out of its tree"
        );
    }

    #[test]
    fn the_walk_order_is_stable_across_runs() {
        let dir = fixture();
        let paths = |entries: Vec<WalkEntry>| {
            entries.into_iter().map(|e| e.path).collect::<Vec<_>>()
        };
        let (first, _) = collect(dir.path(), &CancellationToken::new());
        let (second, _) = collect(dir.path(), &CancellationToken::new());

        // Paths and their order, not the whole entry: reading a file can move
        // its access time, and the fixture assertions elsewhere already pin the
        // sizes. What a deterministic scan means is "the same items, in the
        // same order".
        assert_eq!(paths(first), paths(second), "scans must be deterministic");
    }
}
