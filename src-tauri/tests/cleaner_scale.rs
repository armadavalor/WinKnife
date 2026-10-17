//! The cleaner at the scale a real `%TEMP%` reaches.
//!
//! A user's machine with ~86 000 files in `%LOCALAPPDATA%\Temp` walked every
//! pack in under a minute and then never logged `scan finished`: everything
//! *after* the walk was per-finding work the small fixtures never exposed.
//! This file times each phase separately on a synthetic 100 000-file temp
//! folder so the next regression shows up as a number, not as a frozen screen.
//!
//! ```text
//! cleaner::scan     walk + match + dedupe   <- measured
//! cleaner::analyze  gates + score           <- measured
//! serde_json        the IPC payload         <- measured
//! ```
//!
//! The 100 000-file run is **ignored by default**: it writes a hundred
//! thousand files (once; the tree is reused). Run it deliberately:
//!
//! ```text
//! cargo test --release --test cleaner_scale -- --ignored --nocapture
//! ```
//!
//! A 5 000-file run is not ignored: it is the CI regression guard, with a time
//! budget wide enough for a debug build on a shared runner and narrow enough
//! that going back to per-pack walks or sequential gates breaks it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;
use uuid::Uuid;
use winknife_lib::core::brain::{rules, Brain, ProtectedPaths};
use winknife_lib::core::path_compare::CanonicalPath;
use winknife_lib::core::platform::{
    CachedProcessProbe, MapEnv, StubRecycleBin, SystemEnv, SystemLockProbe, SystemProcessProbe,
};
use winknife_lib::core::scanner::CancellationToken;
use winknife_lib::core::state::ScanResult;
use winknife_lib::modules::cleaner::{self, CleanerContext};

/// Files in the synthetic temp folder of the deliberate run.
const FILES: usize = 100_000;
/// Files in the CI guard's synthetic temp folder.
const CI_FILES: usize = 5_000;
/// Scan + analyze budget for [`CI_FILES`] in a debug build.
const CI_BUDGET: Duration = Duration::from_secs(BUDGET_SECS);
const BUDGET_SECS: u64 = 20;
/// Files per directory: real temp folders are a few huge flat directories plus
/// many small installer/extract folders.
const FILES_PER_DIR: usize = 500;
/// Old enough for the 24-hour rule and the one-hour quiet period.
const AGE: Duration = Duration::from_secs(3 * 24 * 3600);

/// Build the tree once, or reuse it. Under `CARGO_TARGET_TMPDIR`, never the
/// developer's real `%TEMP%`: a cleaner test must not plant junk where the
/// cleaner would later find it.
fn synthetic_temp(files: usize) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cleaner-{files}"));
    let temp = root.join("Local").join("Temp");
    let marker = root.join("COMPLETE");
    if marker.is_file() {
        return root;
    }

    let old = SystemTime::now() - AGE;
    for dir in 0..files / FILES_PER_DIR {
        let path = temp.join(format!("d{dir:04}"));
        fs::create_dir_all(&path).expect("fixture directory");
        for file in 0..FILES_PER_DIR {
            let ext = ["tmp", "log", "dat", "etl", "txt"][file % 5];
            let target = path.join(format!("f{file:03}.{ext}"));
            fs::write(&target, vec![b'x'; 64 + (file * 37) % 2048]).expect("fixture file");
            fs::File::options()
                .write(true)
                .open(&target)
                .and_then(|handle| {
                    handle.set_times(fs::FileTimes::new().set_modified(old).set_accessed(old))
                })
                .expect("backdate fixture file");
        }
    }
    fs::write(&marker, b"complete").expect("marker");
    root
}

fn repo_rules_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("rules")
}

/// Packs pointed at the fixture: `%TEMP%` and `%LOCALAPPDATA%\Temp` name the
/// same tree, exactly as on a stock install.
fn packs(root: &Path) -> Vec<rules::RulePack> {
    let env = MapEnv::new()
        .with("TEMP", root.join("Local").join("Temp").to_string_lossy())
        .with("LOCALAPPDATA", root.join("Local").to_string_lossy());
    rules::load_dir(&repo_rules_dir(), &env).expect("the shipped packs must load")
}

/// The real protected list, from the real environment: the gates must cost
/// what they cost in the app.
fn protected(root: &Path) -> ProtectedPaths {
    ProtectedPaths::from_env(&SystemEnv, &root.join("appdata"), &root.join("quarantine"))
}

fn ms(duration: Duration) -> u128 {
    duration.as_millis()
}

#[test]
#[ignore = "writes 100 000 files; run with --ignored --nocapture"]
fn a_hundred_thousand_temp_files_scan_analyze_and_serialise() {
    let root = synthetic_temp(FILES);
    let packs = packs(&root);
    let protected = protected(&root);
    let brain = Brain::builder()
        .protected_paths(protected.clone())
        .packs(packs.clone())
        .lock_probe(Arc::new(SystemLockProbe))
        .process_probe(Arc::new(CachedProcessProbe::new(Arc::new(
            SystemProcessProbe,
        ))))
        .build();
    let settings = cleaner::CleanerSettings::default();
    let cancel = CancellationToken::new();
    let bin = StubRecycleBin::default();

    // The bare walk, so the scan number below can be split into "reading the
    // directory" and "deciding about what was read".
    let started = Instant::now();
    let mut walked = 0_usize;
    winknife_lib::core::scanner::walk(&root.join("Local").join("Temp"), &cancel, &mut |_| {
        walked += 1;
    });
    let walk_once = started.elapsed();

    let started = Instant::now();
    let output = cleaner::scan(
        &CleanerContext {
            packs: &packs,
            settings: &settings,
            protected: &protected,
            recycle_bin: &bin,
            now: Utc::now(),
            cancel: &cancel,
        },
        None,
    );
    let scanned = started.elapsed();

    // What `dedupe_by_path` pays inside `scan`: one canonicalisation per hit.
    let started = Instant::now();
    for finding in &output.findings {
        std::hint::black_box(CanonicalPath::resolved(&finding.path).key());
    }
    let canonicalised = started.elapsed();

    let started = Instant::now();
    let recommendations = cleaner::analyze(&output.findings, &brain);
    let analyzed = started.elapsed();

    let findings = output.findings.len();
    let result = ScanResult {
        scan_id: Uuid::new_v4(),
        module: "cleaner".to_owned(),
        findings: output.findings,
        recommendations,
        skipped_rules: output.skipped_rules,
        cancelled: false,
    };
    let started = Instant::now();
    let payload = serde_json::to_vec(&result).expect("serialise");
    let serialised = started.elapsed();

    println!(
        "cleaner scale: {findings} findings | one bare walk {} ms ({walked} entries) | scan {} ms (canonicalise alone {} ms) | analyze {} ms | serialise {} ms ({} MB)",
        ms(walk_once),
        ms(scanned),
        ms(canonicalised),
        ms(analyzed),
        ms(serialised),
        payload.len() / 1_000_000
    );
    assert_eq!(findings, FILES, "every fixture file is old temp junk");
}

/// The CI guard: scan and analyze a 5 000-file temp folder inside a budget.
#[test]
fn five_thousand_temp_files_scan_and_analyze_within_budget() {
    let root = synthetic_temp(CI_FILES);
    let packs = packs(&root);
    let protected = protected(&root);
    let brain = Brain::builder()
        .protected_paths(protected.clone())
        .packs(packs.clone())
        .lock_probe(Arc::new(SystemLockProbe))
        .process_probe(Arc::new(CachedProcessProbe::new(Arc::new(
            SystemProcessProbe,
        ))))
        .build();
    let settings = cleaner::CleanerSettings::default();
    let cancel = CancellationToken::new();
    let bin = StubRecycleBin::default();

    let started = Instant::now();
    let output = cleaner::scan(
        &CleanerContext {
            packs: &packs,
            settings: &settings,
            protected: &protected,
            recycle_bin: &bin,
            now: Utc::now(),
            cancel: &cancel,
        },
        None,
    );
    let scanned = started.elapsed();
    let recommendations = cleaner::analyze(&output.findings, &brain);
    let total = started.elapsed();

    println!(
        "cleaner ci guard: {} findings | scan {} ms | scan + analyze {} ms (budget {} ms)",
        output.findings.len(),
        ms(scanned),
        ms(total),
        ms(CI_BUDGET)
    );
    assert_eq!(
        output.findings.len(),
        CI_FILES,
        "every fixture file is old temp junk"
    );
    assert_eq!(recommendations.len(), CI_FILES, "every finding is judged");
    assert!(
        total < CI_BUDGET,
        "scan + analyze of {CI_FILES} temp files took {} ms, over the {} ms budget",
        ms(total),
        ms(CI_BUDGET)
    );
}
