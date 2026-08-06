//! The phase-1 acceptance path, end to end: fixture scan → deterministic
//! findings → clean → files in quarantine with ledger rows → restore →
//! byte-identical tree.
//!
//! # What this test may and may not touch
//!
//! Everything happens inside one tempdir. The fixture is a miniature dirty
//! Windows profile, and the shipped rule packs are pointed at it by handing the
//! loader a [`MapEnv`] instead of the real environment — so `%TEMP%` here means
//! "the fixture", never the developer's actual temp folder (CONVENTIONS.md:
//! "never scan the real machine in tests").
//!
//! Integration tests link the library **without** `cfg(test)`, so the guards
//! inside `SystemPower` and `WindowsRecycleBin` are not active here. That is
//! exactly why this file never names them: the recycle bin is a
//! [`StubRecycleBin`] and no power controller is constructed at all.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::Utc;
use cleanify_lib::core::brain::{rules, Brain, ProtectedPaths};
use cleanify_lib::core::db;
use cleanify_lib::core::executor::{self, ExecEnvironment};
use cleanify_lib::core::platform::{MapEnv, StubRecycleBin, SystemLockProbe};
use cleanify_lib::core::quarantine::{Quarantine, RetentionPolicy};
use cleanify_lib::core::scanner::CancellationToken;
use cleanify_lib::core::types::{ExecOutcome, Finding, RiskBucket};
use cleanify_lib::modules::cleaner::{self, CleanerContext};

/// A miniature dirty Windows profile plus the services that operate on it.
struct World {
    _dir: tempfile::TempDir,
    profile: PathBuf,
    temp: PathBuf,
    explorer: PathBuf,
    planted_windows: PathBuf,
    env: MapEnv,
    conn: rusqlite::Connection,
    quarantine: Quarantine,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let profile = root.join("profile");
        let temp = profile.join("AppData/Local/Temp");
        let explorer = profile.join("AppData/Local/Microsoft/Windows/Explorer");
        // The protected-path injection: a `C:\Windows` stand-in inside the
        // fixture, which must land red however the rules read.
        let planted_windows = profile.join("Windows");

        for path in [&temp, &explorer, &planted_windows] {
            fs::create_dir_all(path).expect("fixture dirs");
        }

        let env = MapEnv::new().with("TEMP", temp.to_string_lossy()).with(
            "LOCALAPPDATA",
            profile.join("AppData/Local").to_string_lossy(),
        );

        Self {
            conn: db::open_in_memory().expect("db"),
            quarantine: Quarantine::with_root(root.join("quarantine"), RetentionPolicy::default()),
            profile,
            temp,
            explorer,
            planted_windows,
            env,
            _dir: dir,
        }
    }

    fn brain(&self) -> Brain {
        Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([
                self.planted_windows.clone(),
                self.quarantine.root().to_path_buf(),
            ]))
            .packs(self.packs())
            .lock_probe(Arc::new(SystemLockProbe))
            .build()
    }

    fn packs(&self) -> Vec<rules::RulePack> {
        rules::load_dir(&repo_rules_dir(), &self.env).expect("the shipped packs must load")
    }

    fn scan(&self, bin: &StubRecycleBin) -> cleaner::ScanOutput {
        let packs = self.packs();
        let cancel = CancellationToken::new();
        cleaner::scan(
            &CleanerContext {
                packs: &packs,
                recycle_bin: bin,
                now: Utc::now(),
                cancel: &cancel,
            },
            None,
        )
    }
}

fn repo_rules_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("rules")
}

/// Write a file and backdate it, so age-based rules can be exercised without
/// waiting a day.
fn aged(path: &Path, bytes: &[u8], hours: u64) -> PathBuf {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent");
    }
    fs::write(path, bytes).expect("write");
    let when = SystemTime::now() - Duration::from_secs(hours * 3600);
    let times = fs::FileTimes::new().set_modified(when).set_accessed(when);
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open")
        .set_times(times)
        .expect("backdate");
    path.to_path_buf()
}

/// Every file under `root`, with its bytes. The restore assertion compares two
/// of these, which is what "byte-identical tree" means.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(dir: &Path, base: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, base, out);
            } else if let Ok(bytes) = fs::read(&path) {
                let relative = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
                out.push((relative, bytes));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

fn bucket_of(
    recommendations: &[cleanify_lib::core::types::Recommendation],
    finding: &Finding,
) -> RiskBucket {
    recommendations
        .iter()
        .find(|rec| rec.finding_ids.contains(&finding.id))
        .map(|rec| rec.risk)
        .unwrap_or_else(|| panic!("no recommendation for {}", finding.path.display()))
}

fn find_by_suffix<'a>(findings: &'a [Finding], suffix: &str) -> &'a Finding {
    findings
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with(suffix)
        })
        .unwrap_or_else(|| panic!("`{suffix}` was not found by the scan"))
}

/// The headline acceptance item: scan, clean, restore, and the tree that comes
/// back is the tree that left.
#[test]
fn scan_clean_and_restore_round_trips_byte_identically() {
    let world = World::new();
    let bin = StubRecycleBin::default();

    aged(&world.temp.join("old-installer.tmp"), &vec![7u8; 4_096], 48);
    aged(
        &world.temp.join("nested/session.dat"),
        b"nested payload",
        72,
    );
    aged(
        &world.explorer.join("iconcache_96.db"),
        &vec![3u8; 2_048],
        30,
    );
    // Left alone: too young for the pack and for the quiet period.
    aged(&world.temp.join("in-flight.tmp"), b"still writing", 0);

    let before = snapshot(&world.profile);
    let brain = world.brain();

    // 1. Scan — read-only.
    let output = world.scan(&bin);
    assert_eq!(
        snapshot(&world.profile),
        before,
        "scan() must not modify anything"
    );

    let recommendations = cleaner::analyze(&output.findings, &brain);
    let approved: Vec<_> = recommendations
        .iter()
        .filter(|rec| rec.risk == RiskBucket::Safe)
        .map(|rec| rec.id)
        .collect();
    assert_eq!(approved.len(), 3, "three green items were expected");

    // 2. Clean.
    let units = executor::plan(&approved, &recommendations, &output.findings);
    let report = executor::execute(
        &units,
        &ExecEnvironment {
            quarantine: &world.quarantine,
            conn: &world.conn,
            brain: &brain,
            recycle_bin: &bin,
        },
        None,
    )
    .expect("execute");

    assert_eq!(report.succeeded(), 3);
    assert_eq!(report.failed(), 0);
    assert_eq!(report.reclaimed_bytes, 4_096 + 14 + 2_048);
    let batch = report.batch_id.expect("a batch id once something moved");

    // Files are gone from the profile and present in quarantine, with rows.
    assert!(!world.temp.join("old-installer.tmp").exists());
    assert!(!world.explorer.join("iconcache_96.db").exists());
    assert!(
        world.temp.join("in-flight.tmp").exists(),
        "an unapproved item must be untouched"
    );

    let items = world
        .quarantine
        .items_in_batch(&world.conn, batch)
        .expect("ledger rows");
    assert_eq!(items.len(), 3, "one ledger row per moved file");
    for item in &items {
        assert!(
            item.quarantine_path.is_file(),
            "the copy must exist on disk"
        );
        assert!(item.quarantine_path.starts_with(world.quarantine.root()));
        assert!(item.is_held());
    }
    assert_eq!(
        world
            .quarantine
            .stats(&world.conn)
            .expect("stats")
            .item_count,
        3
    );

    // 3. Restore.
    let (restored, failed) = world
        .quarantine
        .restore_batch(&world.conn, batch)
        .expect("restore");
    assert_eq!(restored.len(), 3);
    assert!(failed.is_empty());
    assert!(
        restored.iter().all(|outcome| !outcome.renamed),
        "nothing occupied the original paths, so nothing should be renamed"
    );

    assert_eq!(
        snapshot(&world.profile),
        before,
        "the restored tree must be byte-identical to the original"
    );
    assert_eq!(
        world
            .quarantine
            .stats(&world.conn)
            .expect("stats")
            .item_count,
        0
    );
}

/// The protected-path fixture: a `C:\Windows` stand-in inside the fixture root
/// lands red and is never acted on, even when the recommendation is approved.
#[test]
fn a_protected_path_lands_red_and_survives_an_approved_clean() {
    let world = World::new();
    let bin = StubRecycleBin::default();
    let brain = world.brain();

    let victim = aged(
        &world.planted_windows.join("System32/critical.sys"),
        b"do not touch this",
        200,
    );

    // Surface it the way a broader pack would, tagged with a Safe rule.
    let mut finding = Finding::new(
        "cleaner",
        &victim,
        cleanify_lib::core::types::FindingKind::File,
    );
    finding.size_bytes = 17;
    finding.meta = serde_json::json!({
        "packId": "windows-temp",
        "packName": "Windows Temp Files",
        "ruleId": "user-temp-files",
    });

    let recommendations = cleaner::analyze(std::slice::from_ref(&finding), &brain);
    assert_eq!(
        bucket_of(&recommendations, &finding),
        RiskBucket::DoNotDelete
    );
    assert_eq!(
        recommendations[0].action,
        cleanify_lib::core::types::Action::InfoOnly
    );
    assert_eq!(recommendations[0].savings_bytes, 0);

    // Approve it anyway — the executor must still refuse.
    let approved = vec![recommendations[0].id];
    let units = executor::plan(&approved, &recommendations, std::slice::from_ref(&finding));
    let report = executor::execute(
        &units,
        &ExecEnvironment {
            quarantine: &world.quarantine,
            conn: &world.conn,
            brain: &brain,
            recycle_bin: &bin,
        },
        None,
    )
    .expect("execute");

    assert!(matches!(
        report.items[0].outcome,
        ExecOutcome::Skipped { .. }
    ));
    assert!(victim.exists(), "a protected file must never move");
    assert_eq!(report.reclaimed_bytes, 0);
    assert_eq!(report.batch_id, None);
}

/// The locked-file fixture: a real handle held with no sharing, a per-item
/// skip, and the rest of the batch still runs.
#[cfg(windows)]
#[test]
fn a_locked_file_is_skipped_with_a_per_item_report() {
    use std::os::windows::fs::OpenOptionsExt;

    let world = World::new();
    let bin = StubRecycleBin::default();
    let brain = world.brain();

    let locked = aged(&world.temp.join("held-open.tmp"), b"a program has this", 48);
    let free = aged(&world.temp.join("free.tmp"), b"movable", 48);

    // Exactly what a running program does to a file it is using.
    let _held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .expect("hold the file open exclusively");

    let output = world.scan(&bin);
    let recommendations = cleaner::analyze(&output.findings, &brain);

    // The brain's lock gate already puts it in the red bucket at analyze time.
    let locked_finding = find_by_suffix(&output.findings, "held-open.tmp");
    assert_eq!(
        bucket_of(&recommendations, locked_finding),
        RiskBucket::DoNotDelete,
        "a file in use must never be offered as safe"
    );

    // Approve everything, including the locked item, and check the report.
    let approved: Vec<_> = recommendations.iter().map(|rec| rec.id).collect();
    let units = executor::plan(&approved, &recommendations, &output.findings);
    let report = executor::execute(
        &units,
        &ExecEnvironment {
            quarantine: &world.quarantine,
            conn: &world.conn,
            brain: &brain,
            recycle_bin: &bin,
        },
        None,
    )
    .expect("execute");

    let locked_result = report
        .items
        .iter()
        .find(|item| item.path == locked)
        .expect("the locked item must appear in the report");
    match &locked_result.outcome {
        ExecOutcome::Skipped { reason } => {
            assert!(!reason.trim().is_empty(), "a skip must explain itself");
        }
        other => panic!("expected a skip for the locked file, got {other:?}"),
    }

    assert!(locked.exists(), "a locked file must be left where it is");
    assert!(!free.exists(), "the rest of the batch must still run");
    assert!(report.succeeded() >= 1);
}

/// Two identical runs over an unchanged fixture produce the same findings, in
/// the same order, in the same buckets.
#[test]
fn a_fixture_scan_is_deterministic() {
    let world = World::new();
    let bin = StubRecycleBin::new(4, 8_192);
    let brain = world.brain();

    for index in 0..5 {
        aged(
            &world.temp.join(format!("file-{index}.tmp")),
            format!("payload {index}").as_bytes(),
            48,
        );
    }
    aged(&world.explorer.join("thumbcache_1024.db"), b"thumbs", 48);

    let describe = || {
        let output = world.scan(&bin);
        let recommendations = cleaner::analyze(&output.findings, &brain);
        output
            .findings
            .iter()
            .map(|finding| {
                (
                    finding.path.clone(),
                    finding.size_bytes,
                    bucket_of(&recommendations, finding),
                )
            })
            .collect::<Vec<_>>()
    };

    let first = describe();
    assert!(!first.is_empty(), "the fixture should produce findings");
    assert_eq!(first, describe(), "scans must be reproducible");
    assert!(
        first
            .iter()
            .all(|(_, _, bucket)| *bucket == RiskBucket::Safe),
        "every fixture item is in a green pack"
    );
}

/// Emptying the recycle bin is the one action that does not go to quarantine.
/// The stub proves the call is made and that nothing is moved for it.
#[test]
fn emptying_the_recycle_bin_bypasses_quarantine() {
    let world = World::new();
    let bin = StubRecycleBin::new(9, 65_536);
    let brain = world.brain();

    let output = world.scan(&bin);
    let recommendations = cleaner::analyze(&output.findings, &brain);
    let bin_finding = find_by_suffix(&output.findings, "shell:RecycleBinFolder");
    assert_eq!(bin_finding.size_bytes, 65_536);
    assert_eq!(bucket_of(&recommendations, bin_finding), RiskBucket::Safe);

    let approved: Vec<_> = recommendations
        .iter()
        .filter(|rec| rec.finding_ids.contains(&bin_finding.id))
        .map(|rec| rec.id)
        .collect();
    let units = executor::plan(&approved, &recommendations, &output.findings);
    let report = executor::execute(
        &units,
        &ExecEnvironment {
            quarantine: &world.quarantine,
            conn: &world.conn,
            brain: &brain,
            recycle_bin: &bin,
        },
        None,
    )
    .expect("execute");

    assert_eq!(bin.empty_calls(), 1, "the shell API must be asked once");
    assert_eq!(report.succeeded(), 1);
    assert_eq!(report.reclaimed_bytes, 65_536);
    assert_eq!(
        report.batch_id, None,
        "the recycle bin moves nothing into quarantine"
    );
    assert_eq!(
        world
            .quarantine
            .stats(&world.conn)
            .expect("stats")
            .item_count,
        0
    );
}

/// A restore that would overwrite a newer file puts the old one alongside it
/// instead, and says so.
#[test]
fn restoring_over_a_recreated_file_never_overwrites_it() {
    let world = World::new();
    let bin = StubRecycleBin::default();
    let brain = world.brain();

    let path = aged(&world.temp.join("recreated.tmp"), b"original bytes", 48);
    let output = world.scan(&bin);
    let recommendations = cleaner::analyze(&output.findings, &brain);
    let approved: Vec<_> = recommendations.iter().map(|rec| rec.id).collect();
    let units = executor::plan(&approved, &recommendations, &output.findings);
    let report = executor::execute(
        &units,
        &ExecEnvironment {
            quarantine: &world.quarantine,
            conn: &world.conn,
            brain: &brain,
            recycle_bin: &bin,
        },
        None,
    )
    .expect("execute");
    let batch = report.batch_id.expect("batch");

    // The application that owned the file recreated it while it was away.
    fs::write(&path, b"brand new bytes").expect("recreate");

    let (restored, failed) = world
        .quarantine
        .restore_batch(&world.conn, batch)
        .expect("restore");

    assert!(failed.is_empty());
    assert_eq!(restored.len(), 1);
    assert!(restored[0].renamed, "a collision must be reported");
    assert_eq!(
        fs::read(&path).expect("read"),
        b"brand new bytes",
        "the newer file must survive untouched"
    );
    assert_eq!(
        fs::read(&restored[0].restored_to).expect("read"),
        b"original bytes"
    );
}

/// Retention and the size cap, driven through the same public API the app
/// calls at startup.
#[test]
fn expired_items_are_purged_and_the_cap_is_enforced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let work = dir.path().join("work");
    fs::create_dir_all(&work).expect("work");
    let conn = db::open_in_memory().expect("db");
    let quarantine = Quarantine::with_root(
        dir.path().join("quarantine"),
        RetentionPolicy {
            retention_days: 7,
            max_bytes: 25,
        },
    );

    let batch = quarantine.start_batch();
    let mut items = Vec::new();
    for index in 0..4 {
        let path = work.join(format!("f{index}.bin"));
        fs::write(&path, vec![b'x'; 10]).expect("write");
        let item = quarantine.take(&conn, batch, &path).expect("take");
        conn.execute(
            "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
            rusqlite::params![
                item.id.to_string(),
                (Utc::now() - chrono::Duration::minutes(10 - index)).to_rfc3339()
            ],
        )
        .expect("age");
        items.push(item);
    }

    // The oldest is also past its retention window.
    conn.execute(
        "UPDATE quarantine SET purge_after = ?2 WHERE id = ?1",
        rusqlite::params![
            items[0].id.to_string(),
            (Utc::now() - chrono::Duration::days(1)).to_rfc3339()
        ],
    )
    .expect("backdate");

    let report = quarantine.purge(&conn, Utc::now()).expect("purge");

    assert!(report.failed_items.is_empty());
    assert!(
        quarantine.stats(&conn).expect("stats").total_bytes <= 25,
        "the cap must hold after a purge"
    );
    assert!(
        !items[0].quarantine_path.exists(),
        "the expired item is gone"
    );
    assert!(
        items[3].quarantine_path.exists(),
        "the newest item must survive"
    );
    assert_eq!(
        report.purged_items + quarantine.stats(&conn).expect("stats").item_count as u64,
        4
    );
}
