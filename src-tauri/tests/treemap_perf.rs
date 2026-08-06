//! Phase 3, task 5: the 100 000-file synthetic tree.
//!
//! Module spec 04's acceptance list says "100k-file synthetic tree: first paint
//! < 3s on dev machine, UI stays responsive (chunked)". This measures the part
//! that is actually expensive and actually measurable from a test binary:
//! everything between the user clicking **Map this drive** and the first
//! rectangles being available to draw.
//!
//! ```text
//! click -> treemap::scan   (walk + aggregate)   <- measured here
//!       -> save_cache      (SQLite write)       <- measured here
//!       -> treemap::children (the first level)  <- measured here
//!       -> IPC to the WebView                   <- not measured
//!       -> squarify + canvas paint              <- measured by npm run test:ui
//! ```
//!
//! The two unmeasured pieces are bounded and small: the level is at most
//! `MAX_CHILDREN + 1` rows of JSON, and `npm run test:ui` times the layout of
//! 400 rectangles at a quarter of a millisecond. The number this test prints is
//! therefore the honest bulk of "first paint", and it is printed rather than
//! merely asserted so a run of it is the evidence.
//!
//! **Ignored by default**: it writes a hundred thousand files, which is not
//! something `cargo test` should do on every run. Run it deliberately:
//!
//! ```text
//! cargo test --release --test treemap_perf -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use winknife_lib::core::db;
use winknife_lib::core::scanner::CancellationToken;
use winknife_lib::modules::treemap;

/// Files in the synthetic tree.
const FILES: usize = 100_000;
/// Files per leaf directory, so the shape resembles a real profile rather than
/// one enormous folder.
const FILES_PER_DIR: usize = 50;
/// The budget module spec 04 sets.
const BUDGET_MS: u128 = 3_000;

/// Build a tree of `FILES` files, or reuse one that is already there.
///
/// Reused deliberately: creating a hundred thousand files takes far longer than
/// the measurement does, and a warm tree is the fairer thing to measure anyway —
/// the user's drive is not freshly written either.
fn synthetic_tree() -> PathBuf {
    let root = std::env::temp_dir().join("winknife-perf-100k");
    let marker = root.join("COMPLETE");
    if marker.is_file() {
        return root;
    }

    let dirs = FILES / FILES_PER_DIR;
    // Three levels: root/gNN/dNNN/fNN.ext, which is roughly the depth of a real
    // AppData tree.
    for dir in 0..dirs {
        let path = root
            .join(format!("g{:02}", dir / 40))
            .join(format!("d{dir:04}"));
        std::fs::create_dir_all(&path).expect("fixture directory");
        for file in 0..FILES_PER_DIR {
            // Varied sizes and extensions so the categoriser and the size sort
            // both do real work.
            let ext = ["bin", "log", "jpg", "mp4", "rs"][file % 5];
            let size = 64 + (file * 37) % 4096;
            std::fs::write(path.join(format!("f{file:02}.{ext}")), vec![b'x'; size])
                .expect("fixture file");
        }
    }
    std::fs::write(&marker, b"complete").expect("marker");
    root
}

fn count_files(root: &Path) -> usize {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(_) => total += 1,
                Err(_) => {}
            }
        }
    }
    total
}

#[test]
#[ignore = "writes 100k files; run with --ignored"]
fn a_hundred_thousand_files_reach_first_paint_inside_the_budget() {
    let root = synthetic_tree();
    let on_disk = count_files(&root);
    println!("tree: {} files under {}", on_disk, root.display());

    let conn = db::open_in_memory().expect("db");
    let cancel = CancellationToken::new();

    let started = Instant::now();
    let scan = treemap::scan(&root, &cancel, None);
    let walk_ms = started.elapsed().as_millis();

    let cached_at = Instant::now();
    treemap::save_cache(&conn, &scan).expect("cache");
    let cache_ms = cached_at.elapsed().as_millis();

    let level_at = Instant::now();
    let oracle = treemap::MemoryOracle::new(&scan);
    let level = treemap::children(&root, &root, &oracle).expect("level");
    let level_ms = level_at.elapsed().as_millis();

    let total_ms = started.elapsed().as_millis();
    let root_total = scan.root_total().expect("root aggregate");

    println!("walk + aggregate : {walk_ms} ms");
    println!("cache write      : {cache_ms} ms");
    println!("first level      : {level_ms} ms");
    println!("TOTAL to paint   : {total_ms} ms  (budget {BUDGET_MS} ms)");
    println!(
        "measured         : {} files, {} directories, {} bytes",
        root_total.file_count, root_total.dir_count, root_total.total_bytes
    );
    println!("first level rows : {}", level.children.len());

    assert_eq!(
        root_total.file_count as usize, on_disk,
        "the scan must account for every file that is really there"
    );
    assert!(
        total_ms < BUDGET_MS,
        "first paint took {total_ms} ms, over the {BUDGET_MS} ms budget"
    );
}

/// Zooming has to stay instant whatever the drive holds — that is the whole
/// point of serving one directory at a time.
#[test]
#[ignore = "depends on the 100k fixture; run with --ignored"]
fn zooming_into_a_folder_is_a_single_directory_read() {
    let root = synthetic_tree();
    let conn = db::open_in_memory().expect("db");
    treemap::save_cache(
        &conn,
        &treemap::scan(&root, &CancellationToken::new(), None),
    )
    .expect("cache");

    let deep = root.join("g00").join("d0000");
    let started = Instant::now();
    let level =
        treemap::children(&deep, &root, &treemap::CacheOracle::new(&conn, &root)).expect("level");
    let ms = started.elapsed().as_millis();

    println!(
        "zoom into one folder: {ms} ms, {} rows",
        level.children.len()
    );
    assert_eq!(level.children.len(), 50);
    assert!(
        ms < 250,
        "zooming took {ms} ms, which would feel like a wait"
    );
}

/// Cancellation has to actually stop the walk, not merely be offered.
///
/// **Not ignored.** It used to be, on the strength of the 100 000-file fixture —
/// so cancellation-under-load, the one behaviour a user reaches for when a scan
/// is taking too long, never ran on a default `cargo test` (security review
/// L16). It builds its own cheap fixture instead: 12 000 files is enough to be
/// walking when the token trips and costs a fraction of a second to create.
#[test]
fn cancelling_a_big_walk_stops_it_promptly() {
    let root = cancellable_tree();
    let cancel = CancellationToken::new();

    // Trip the token once a few thousand entries have gone past, from inside the
    // progress sink — which is exactly where the UI's stop button lands.
    let mut seen = 0u64;
    let mut sink = |progress: &treemap::TreeProgress<'_>| {
        seen = progress.entries;
        if seen >= 3_000 {
            cancel.cancel();
        }
    };

    let started = Instant::now();
    let scan = treemap::scan(&root, &cancel, Some(&mut sink));
    let ms = started.elapsed().as_millis();

    println!("cancelled after {seen} entries in {ms} ms");
    assert!(scan.cancelled, "the walk must report that it was stopped");
    assert!(
        scan.dirs.len() < 2_000,
        "a cancelled walk kept going: {} directories",
        scan.dirs.len()
    );
}

/// Files in the cheap fixture the cancellation test uses.
const CANCELLABLE_FILES: usize = 12_000;

/// A tree big enough that a walk is still going when the token trips, and small
/// enough to build on every run. Empty files: the walk never reads contents.
fn cancellable_tree() -> PathBuf {
    let root = std::env::temp_dir().join("winknife-perf-cancel");
    let marker = root.join("COMPLETE");
    if marker.is_file() {
        return root;
    }

    for dir in 0..(CANCELLABLE_FILES / 100) {
        let path = root.join(format!("d{dir:03}"));
        std::fs::create_dir_all(&path).expect("fixture directory");
        for file in 0..100 {
            std::fs::write(path.join(format!("f{file:03}.bin")), []).expect("fixture file");
        }
    }
    std::fs::write(&marker, b"complete").expect("marker");
    root
}

/* -------------------------------------------------------------------------
 * The other axis: depth (security review M21)
 * ---------------------------------------------------------------------- */

/// How deep the chain goes. Beyond `MAX_PATH`, deliberately — a `node_modules`
/// tree or a deeply nested build output reaches this on real machines, and it is
/// the axis the wide/shallow fixtures above never touched.
const CHAIN_DEPTH: usize = 3_000;

/// The budget. The aggregation wraps the walk, so it may cost a small multiple
/// of it — not fourteen times it, which is what a per-directory O(depth) key
/// cost before the arena was rekeyed: walk 5.34 s, `treemap::scan` 75.3 s.
const DEPTH_OVERHEAD_BUDGET: f64 = 3.0;

/// A single chain of `CHAIN_DEPTH` nested directories with one file at the
/// bottom, created through the verbatim `\\?\` prefix so `MAX_PATH` does not
/// stop it.
fn deep_chain() -> Option<PathBuf> {
    let root = std::env::temp_dir().join("winknife-perf-deep");
    // Beside the tree, not inside it: the test asserts on the exact number of
    // bytes and directories the chain holds, and a marker file in the root would
    // be counted.
    let marker = std::env::temp_dir().join("winknife-perf-deep.COMPLETE");
    if marker.is_file() {
        return Some(root);
    }

    // `\\?\` turns off Win32 path parsing, which is the only way to create a
    // path this long. Everything after creation uses the ordinary spelling.
    let mut building = PathBuf::from(format!("\\\\?\\{}", root.display()));
    std::fs::create_dir_all(&building).ok()?;
    for level in 0..CHAIN_DEPTH {
        building.push(format!("d{level:04}"));
        if std::fs::create_dir_all(&building).is_err() {
            println!("this machine would not create a {CHAIN_DEPTH}-deep chain");
            return None;
        }
    }
    std::fs::write(building.join("leaf.bin"), vec![b'z'; 4_096]).ok()?;
    std::fs::write(
        PathBuf::from(format!("\\\\?\\{}", marker.display())),
        b"complete",
    )
    .ok()?;
    Some(root)
}

/// The depth axis of the same question the 100k test asks about width.
///
/// Ignored by default for the same reason its siblings are — it writes three
/// thousand directories — but it is the case that found M21, and it is measured
/// against the walk it wraps rather than against a wall-clock number, so the
/// assertion means the same thing on a slow machine as on a fast one.
#[test]
#[ignore = "writes a 3000-deep directory chain; run with --ignored"]
fn a_very_deep_tree_costs_a_small_multiple_of_its_walk() {
    let Some(root) = deep_chain() else {
        println!("skipped: no long-path support on this machine");
        return;
    };

    // The walk alone, which is the floor any aggregation has to pay.
    let walked = Instant::now();
    let mut entries = 0u64;
    winknife_lib::core::scanner::walk_with(
        &root,
        winknife_lib::core::scanner::WalkOptions {
            report_links: true,
            accurate_timestamps: false,
        },
        &CancellationToken::new(),
        &mut |_| entries += 1,
    );
    let walk_ms = walked.elapsed().as_millis().max(1);

    let scanned = Instant::now();
    let scan = treemap::scan(&root, &CancellationToken::new(), None);
    let scan_ms = scanned.elapsed().as_millis().max(1);

    let overhead = scan_ms as f64 / walk_ms as f64;
    println!("depth            : {CHAIN_DEPTH}");
    println!("walk alone       : {walk_ms} ms  ({entries} entries)");
    println!("treemap::scan    : {scan_ms} ms");
    println!("overhead         : {overhead:.2}x  (budget {DEPTH_OVERHEAD_BUDGET:.1}x)");

    assert_eq!(
        scan.dirs.len(),
        CHAIN_DEPTH + 1,
        "one node per directory plus the root"
    );
    assert_eq!(
        scan.root_total().expect("root aggregate").total_bytes,
        4_096,
        "and the leaf's bytes have to reach the root"
    );
    assert!(
        overhead < DEPTH_OVERHEAD_BUDGET,
        "aggregating a {CHAIN_DEPTH}-deep tree took {overhead:.2}x its own walk \
         ({scan_ms} ms around {walk_ms} ms)"
    );
}
