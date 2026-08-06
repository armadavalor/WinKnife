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
#[test]
#[ignore = "depends on the 100k fixture; run with --ignored"]
fn cancelling_a_big_walk_stops_it_promptly() {
    let root = synthetic_tree();
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
