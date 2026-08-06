//! Turns approved recommendations into a quarantine batch and a report.
//!
//! Why a separate layer: modules decide *what* to suggest, the executor decides
//! *how* it happens, and there is exactly one "how" — quarantine. A module
//! cannot delete anything even if it wanted to, because `execute` hands the
//! executor a list and gets a report back.
//!
//! Three rules this file enforces, in order, for every single item:
//!
//! 1. The recommendation must still validate (no mutating action on a red item).
//! 2. The path must not be protected (re-checked here, not trusted from the
//!    analyze phase — the UI could have been sitting open for an hour).
//! 3. The file must not be in use. Locked means *skipped and reported*, never
//!    failed silently and never a half-finished batch.
//!
//! Every item gets a `tracing` line whatever happens, because this app deletes
//! things and an audit trail is the difference between a bug and a mystery.

use std::collections::HashMap;

use rusqlite::Connection;
use uuid::Uuid;

use crate::core::brain::{self, Brain, GateVerdict};
use crate::core::error::Result;
use crate::core::platform::RecycleBin;
use crate::core::quarantine::Quarantine;
use crate::core::types::{
    Action, ExecItemResult, ExecOutcome, ExecReport, Finding, FindingKind, Recommendation,
};

/// One unit of work: a recommendation paired with the finding it acts on.
#[derive(Debug, Clone, Copy)]
pub struct ExecUnit<'a> {
    /// The approved recommendation.
    pub recommendation: &'a Recommendation,
    /// The finding it refers to.
    pub finding: &'a Finding,
}

/// Everything the executor is allowed to touch.
pub struct ExecEnvironment<'a> {
    /// Where removed items go.
    pub quarantine: &'a Quarantine,
    /// The ledger.
    pub conn: &'a Connection,
    /// Re-checks protected paths and locks at execute time.
    pub brain: &'a Brain,
    /// The recycle bin, for the one action that is not a quarantine move.
    pub recycle_bin: &'a dyn RecycleBin,
}

/// Per-request confirmations, separate from the services in
/// [`ExecEnvironment`] because they belong to one click, not to the app.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecOptions {
    /// Whether the user confirmed emptying the recycle bin *specifically*.
    ///
    /// Everything else in a batch goes to quarantine and can be undone; this
    /// one is final and takes every drive with it. It used to trigger off a
    /// bare `Uuid` in `approved`, exactly like a quarantine item, so nothing on
    /// the Rust side could tell an informed confirmation from a stray id
    /// (security review H6). Without this flag a recycle-bin item is skipped
    /// and says why.
    pub confirm_empty_recycle_bin: bool,
}

/// What has already happened in this batch.
#[derive(Debug, Default)]
struct BatchState {
    /// Whether anything reached quarantine, so the report only names a batch
    /// that exists.
    moved_anything: bool,
    /// Whether the recycle bin has already been emptied. Two recycle-bin items
    /// in one batch used to empty it twice and count the bytes twice.
    emptied_bin: bool,
}

/// Progress callback so the UI can show which item is being handled.
pub type ProgressSink<'a> = &'a mut dyn FnMut(u64, u64, &Finding);

/// Apply `units`, isolating every failure to its own item.
///
/// Returns a report with one entry per unit; the batch never aborts early.
///
/// # Errors
/// Returns an error only if the ledger itself is unusable. Anything
/// item-specific lands in the report.
pub fn execute(
    units: &[ExecUnit<'_>],
    env: &ExecEnvironment<'_>,
    options: &ExecOptions,
    mut progress: Option<ProgressSink<'_>>,
) -> Result<ExecReport> {
    if units.is_empty() {
        return Ok(ExecReport::default());
    }

    let total = units.len() as u64;
    // The batch id is allocated up front so the report can name it even if
    // every item ends up skipped.
    let batch = env.quarantine.start_batch();
    let mut state = BatchState::default();
    let mut items = Vec::with_capacity(units.len());

    tracing::info!(batch = %batch, items = total, "execute batch started");

    for (index, unit) in units.iter().enumerate() {
        if let Some(sink) = progress.as_deref_mut() {
            sink(index as u64, total, unit.finding);
        }

        let (outcome, reclaimed) = apply_one(unit, env, options, batch, &mut state);
        match &outcome {
            ExecOutcome::Succeeded => tracing::info!(
                batch = %batch,
                path = %unit.finding.path.display(),
                action = ?unit.recommendation.action,
                reclaimed_bytes = reclaimed,
                "item applied"
            ),
            ExecOutcome::Skipped { reason } => tracing::info!(
                batch = %batch,
                path = %unit.finding.path.display(),
                reason = reason.as_str(),
                "item skipped"
            ),
            ExecOutcome::Failed { error } => tracing::warn!(
                batch = %batch,
                path = %unit.finding.path.display(),
                error = error.as_str(),
                "item failed"
            ),
        }

        items.push(ExecItemResult {
            recommendation_id: unit.recommendation.id,
            finding_id: unit.finding.id,
            path: unit.finding.path.clone(),
            outcome,
            reclaimed_bytes: reclaimed,
        });
    }

    if let Some(sink) = progress {
        if let Some(last) = units.last() {
            sink(total, total, last.finding);
        }
    }

    // A batch id that never received a file would show up as an empty row in
    // the quarantine drawer, so only report one when something actually moved.
    let report = ExecReport::from_items(state.moved_anything.then_some(batch), items);
    tracing::info!(
        batch = %batch,
        succeeded = report.succeeded(),
        failed = report.failed(),
        reclaimed_bytes = report.reclaimed_bytes,
        "execute batch finished"
    );
    Ok(report)
}

/// Handle one unit. Never panics, never propagates: the outcome is the report.
fn apply_one(
    unit: &ExecUnit<'_>,
    env: &ExecEnvironment<'_>,
    options: &ExecOptions,
    batch: Uuid,
    state: &mut BatchState,
) -> (ExecOutcome, u64) {
    let skipped = |reason: String| (ExecOutcome::Skipped { reason }, 0);
    let failed = |error: String| (ExecOutcome::Failed { error }, 0);

    // 1. The recommendation still has to be internally valid.
    if let Err(err) = unit.recommendation.validate() {
        return failed(err.to_string());
    }

    // Non-mutating actions are no-ops by definition; report them honestly
    // rather than pretending work happened.
    if !unit.recommendation.action.is_mutating() {
        return skipped(format!(
            "no action to apply for `{:?}`",
            unit.recommendation.action
        ));
    }

    if unit.recommendation.action != Action::Quarantine {
        return skipped(format!(
            "action `{:?}` is not supported yet",
            unit.recommendation.action
        ));
    }

    // The recycle bin is the documented exception: it has no path to move,
    // because Windows' own bin already is the reversible step. It is also the
    // only irreversible thing in a batch, so it needs its own confirmation
    // rather than riding along on a selected id (security review H6).
    if unit.finding.kind == FindingKind::RecycleBinItem {
        return empty_recycle_bin(unit, env, options, state);
    }

    // 2. Protected paths are re-checked here, not trusted from analyze time,
    //    and against the *canonical* path rather than the text we were handed.
    //    `C:\PROGRA~1\...` and a junction pointing at System32 both name a
    //    protected file while spelling it as something the list has never seen
    //    (security review C1a). Spellings that cannot be canonicalised into a
    //    plain local path at all — `\\localhost\C$\Windows\...`, `\??\C:\...`,
    //    `\\?\GLOBALROOT\...` — are refused by the gate, not guessed at (C2).
    if let Some(reason) = env.brain.protection_reason(&unit.finding.path) {
        tracing::warn!(
            path = %unit.finding.path.display(),
            "refused to act on a protected path at execute time"
        );
        return skipped(reason);
    }
    match brain::gate_path(&unit.finding.path) {
        GateVerdict::Local(canonical) => {
            if let Some(reason) = env.brain.protection_reason(&canonical) {
                tracing::warn!(
                    path = %unit.finding.path.display(),
                    canonical = %canonical.display(),
                    "refused to act on a path that resolves into a protected location"
                );
                return skipped(reason);
            }
        }
        // Nothing is there to protect, and `take` reports the miss properly.
        GateVerdict::Missing => {}
        GateVerdict::Refused(why) => {
            tracing::warn!(
                path = %unit.finding.path.display(),
                reason = why.as_str(),
                "refused to act on a path the gate cannot judge"
            );
            return skipped(format!("skipped: {why}"));
        }
    }

    // 3. Locks are re-checked too: a file can be opened between review and
    //    confirm. Skipped, reported, batch continues.
    let lock = env.brain.lock_state(&unit.finding.path);
    if !lock.is_free() {
        return skipped(format!("skipped: {}", lock.reason()));
    }

    match env.quarantine.take(env.conn, batch, &unit.finding.path) {
        Ok(item) => {
            state.moved_anything = true;
            (ExecOutcome::Succeeded, item.size_bytes)
        }
        Err(err) => failed(err.to_string()),
    }
}

/// Empty the recycle bin, at most once per batch and only when the user said so
/// about this specific action.
fn empty_recycle_bin(
    unit: &ExecUnit<'_>,
    env: &ExecEnvironment<'_>,
    options: &ExecOptions,
    state: &mut BatchState,
) -> (ExecOutcome, u64) {
    let skipped = |reason: &str| {
        (
            ExecOutcome::Skipped {
                reason: reason.to_owned(),
            },
            0,
        )
    };

    if !options.confirm_empty_recycle_bin {
        tracing::warn!("refused to empty the recycle bin: it was not confirmed separately");
        return skipped(
            "skipped: emptying the recycle bin is final and is not covered by the \
             quarantine, so it needs its own confirmation",
        );
    }
    // Two recycle-bin findings in one batch would otherwise empty it twice and
    // count the reclaimed bytes twice.
    if state.emptied_bin {
        return skipped("skipped: the recycle bin was already emptied in this batch");
    }

    // Sized now rather than trusting the analyze-time figure, which can be
    // hours old: the number the report shows is what this call actually freed.
    let before = env
        .recycle_bin
        .query()
        .map(|stats| stats.size_bytes)
        .unwrap_or(unit.finding.size_bytes);

    match env.recycle_bin.empty() {
        Ok(()) => {
            state.emptied_bin = true;
            let after = env
                .recycle_bin
                .query()
                .map(|stats| stats.size_bytes)
                .unwrap_or(0);
            (ExecOutcome::Succeeded, before.saturating_sub(after))
        }
        Err(err) => (
            ExecOutcome::Failed {
                error: err.to_string(),
            },
            0,
        ),
    }
}

/// Pair approved recommendation ids with their findings.
///
/// Ids the caller does not know about are dropped rather than erroring: a stale
/// UI selection should not stop a clean the user asked for.
pub fn plan<'a>(
    approved: &[Uuid],
    recommendations: &'a [Recommendation],
    findings: &'a [Finding],
) -> Vec<ExecUnit<'a>> {
    let by_id: HashMap<Uuid, &Finding> = findings.iter().map(|f| (f.id, f)).collect();
    let approved_set: std::collections::HashSet<Uuid> = approved.iter().copied().collect();

    let mut units = Vec::new();
    for recommendation in recommendations
        .iter()
        .filter(|rec| approved_set.contains(&rec.id))
    {
        for finding_id in &recommendation.finding_ids {
            if let Some(finding) = by_id.get(finding_id) {
                units.push(ExecUnit {
                    recommendation,
                    finding,
                });
            }
        }
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::brain::{Brain, ProtectedPaths};
    use crate::core::db;
    use crate::core::platform::{AlwaysUnlocked, LockProbe, LockState, StubRecycleBin};
    use crate::core::quarantine::RetentionPolicy;
    use crate::core::types::RiskBucket;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    struct Harness {
        _dir: tempfile::TempDir,
        conn: Connection,
        quarantine: Quarantine,
        work: PathBuf,
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).expect("work");
        Harness {
            conn: db::open_in_memory().expect("db"),
            quarantine: Quarantine::with_root(
                dir.path().join("quarantine"),
                RetentionPolicy::default(),
            ),
            work,
            _dir: dir,
        }
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write");
        path
    }

    fn finding_at(path: &Path, size: u64) -> Finding {
        let mut finding = Finding::new("cleaner", path, FindingKind::File);
        finding.size_bytes = size;
        finding
    }

    fn quarantine_rec(finding: &Finding) -> Recommendation {
        Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            95,
            Action::Quarantine,
            "temporary file older than 24h",
            finding.size_bytes,
        )
        .expect("valid recommendation")
    }

    fn brain() -> Brain {
        Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .lock_probe(Arc::new(AlwaysUnlocked))
            .build()
    }

    fn env<'a>(h: &'a Harness, brain: &'a Brain, bin: &'a StubRecycleBin) -> ExecEnvironment<'a> {
        ExecEnvironment {
            quarantine: &h.quarantine,
            conn: &h.conn,
            brain,
            recycle_bin: bin,
        }
    }

    #[test]
    fn an_empty_batch_reports_nothing() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let report =
            execute(&[], &env(&h, &brain, &bin), &ExecOptions::default(), None).expect("execute");
        assert_eq!(report, ExecReport::default());
        assert_eq!(report.batch_id, None);
    }

    #[test]
    fn approved_items_move_to_quarantine_and_report_bytes() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let a = write(&h.work, "a.tmp", &[b'a'; 100]);
        let b = write(&h.work, "b.tmp", &vec![b'b'; 250]);
        let (fa, fb) = (finding_at(&a, 100), finding_at(&b, 250));
        let (ra, rb) = (quarantine_rec(&fa), quarantine_rec(&fb));
        let units = vec![
            ExecUnit {
                recommendation: &ra,
                finding: &fa,
            },
            ExecUnit {
                recommendation: &rb,
                finding: &fb,
            },
        ];

        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert_eq!(report.succeeded(), 2);
        assert_eq!(report.failed(), 0);
        assert_eq!(report.reclaimed_bytes, 350);
        assert!(report.batch_id.is_some());
        assert!(!a.exists() && !b.exists());
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 2);
    }

    #[test]
    fn one_failure_does_not_stop_the_batch() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let good = write(&h.work, "good.tmp", b"payload");
        let missing = h.work.join("never-existed.tmp");
        let (fg, fm) = (finding_at(&good, 7), finding_at(&missing, 7));
        let (rg, rm) = (quarantine_rec(&fg), quarantine_rec(&fm));
        let units = vec![
            ExecUnit {
                recommendation: &rm,
                finding: &fm,
            },
            ExecUnit {
                recommendation: &rg,
                finding: &fg,
            },
        ];

        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert_eq!(report.items.len(), 2);
        assert_eq!(report.failed(), 1);
        assert_eq!(report.succeeded(), 1);
        assert_eq!(report.reclaimed_bytes, 7);
        assert!(!good.exists(), "the healthy item still moved");
    }

    /// The phase-brief acceptance item: a locked file is skipped with a
    /// per-item report, and the rest of the batch carries on.
    #[cfg(windows)]
    #[test]
    fn a_locked_file_is_skipped_and_reported() {
        use crate::core::platform::SystemLockProbe;
        use std::os::windows::fs::OpenOptionsExt;

        let h = harness();
        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .lock_probe(Arc::new(SystemLockProbe))
            .build();
        let bin = StubRecycleBin::default();

        let locked = write(&h.work, "locked.tmp", b"held open");
        let free = write(&h.work, "free.tmp", b"movable");

        // A real handle with no sharing: exactly what a running program does.
        let _held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .expect("hold the file open");

        let (fl, ff) = (finding_at(&locked, 9), finding_at(&free, 7));
        let (rl, rf) = (quarantine_rec(&fl), quarantine_rec(&ff));
        let units = vec![
            ExecUnit {
                recommendation: &rl,
                finding: &fl,
            },
            ExecUnit {
                recommendation: &rf,
                finding: &ff,
            },
        ];

        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        let locked_result = report
            .items
            .iter()
            .find(|i| i.path == locked)
            .expect("the locked item is reported");
        match &locked_result.outcome {
            ExecOutcome::Skipped { reason } => {
                assert!(reason.contains("in use"), "got `{reason}`");
            }
            other => panic!("expected a skip for the locked file, got {other:?}"),
        }

        assert!(locked.exists(), "a locked file must be left alone");
        assert!(!free.exists(), "the rest of the batch must still run");
        assert_eq!(report.succeeded(), 1);
        assert_eq!(report.reclaimed_bytes, 7);
    }

    /// Same guarantee without needing Windows sharing semantics: a probe that
    /// reports everything locked must produce skips, not deletions.
    #[test]
    fn a_lock_probe_reporting_in_use_blocks_every_move() {
        struct AlwaysLocked;
        impl LockProbe for AlwaysLocked {
            fn probe(&self, _path: &Path) -> LockState {
                LockState::InUse
            }
        }

        let h = harness();
        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .lock_probe(Arc::new(AlwaysLocked))
            .build();
        let bin = StubRecycleBin::default();
        let path = write(&h.work, "busy.tmp", b"data");
        let finding = finding_at(&path, 4);
        let rec = quarantine_rec(&finding);

        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert_eq!(report.succeeded(), 0);
        assert!(path.exists());
        assert_eq!(report.batch_id, None, "nothing moved, so no batch");
    }

    #[test]
    fn a_protected_path_is_refused_even_when_approved() {
        let h = harness();
        let protected_dir = h.work.join("Windows");
        std::fs::create_dir_all(&protected_dir).expect("dir");
        let victim = write(&protected_dir, "kernel.sys", b"critical");

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([protected_dir.clone()]))
            .lock_probe(Arc::new(AlwaysUnlocked))
            .build();
        let bin = StubRecycleBin::default();

        // A recommendation that lies about the risk, as a rule pack bug would.
        let finding = finding_at(&victim, 8);
        let rec = Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            100,
            Action::Quarantine,
            "a buggy rule claimed this was safe",
            8,
        )
        .expect("recommendation");

        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert!(matches!(
            report.items[0].outcome,
            ExecOutcome::Skipped { .. }
        ));
        assert!(victim.exists(), "protected files are never moved");
        assert_eq!(report.reclaimed_bytes, 0);
    }

    /// The 8.3 alias of a directory name is a second spelling of the same
    /// place, and the lexical protected-path list cannot see through it. The
    /// gate canonicalises first, so it can (security review C1a/C1f).
    #[cfg(windows)]
    #[test]
    fn an_8_3_alias_of_a_protected_directory_is_refused() {
        let h = harness();
        // A long name with spaces, which is what makes Windows mint an alias.
        let protected_dir = h.work.join("Protected Program Files");
        std::fs::create_dir_all(&protected_dir).expect("dir");
        let victim = write(&protected_dir, "kernel.sys", b"critical");

        // The list is written in long names, as the real one is.
        let long_dir = dunce::canonicalize(&protected_dir).expect("canonicalize");
        let no_alias = match crate::core::test_support::short_path_name(&victim) {
            None => true,
            Some(short) => short == dunce::canonicalize(&victim).expect("canonicalize"),
        };
        if no_alias {
            // M11: 8.3 alias generation is a per-volume setting nothing in this
            // test can turn on, so this skip is real — but it is now visible,
            // and the mock-based companion below asserts the same gate logic on
            // every machine.
            crate::core::test_support::privileged_operation_unavailable(
                "an_8_3_alias_of_a_protected_directory_is_refused",
                "8.3 alias generation is disabled on this volume (fsutil 8dot3name)",
            );
            return;
        }
        let short_victim =
            crate::core::test_support::short_path_name(&victim).expect("checked just above");

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([long_dir]))
            .lock_probe(Arc::new(AlwaysUnlocked))
            .build();
        let bin = StubRecycleBin::default();

        assert!(
            brain.protection_reason(&short_victim).is_none(),
            "the premise of this test is that the short name slips past the lexical list"
        );

        let finding = finding_at(&short_victim, 8);
        let rec = quarantine_rec(&finding);
        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert!(
            matches!(report.items[0].outcome, ExecOutcome::Skipped { .. }),
            "got {:?}",
            report.items[0].outcome
        );
        assert!(
            victim.exists(),
            "a protected file must survive its own alias"
        );
    }

    /// A junction is the other way to spell a protected place as somewhere
    /// harmless. Skipped when the environment cannot create one.
    #[cfg(windows)]
    #[test]
    fn a_link_pointing_into_a_protected_directory_is_refused() {
        let h = harness();
        let protected_dir = h.work.join("Windows");
        std::fs::create_dir_all(&protected_dir).expect("dir");
        let victim = write(&protected_dir, "kernel.sys", b"critical");

        let door = h.work.join("innocent");
        if std::os::windows::fs::symlink_dir(&protected_dir, &door).is_err() {
            // M11: loud, not silent. This used to `return` and pass.
            crate::core::test_support::privileged_operation_unavailable(
                "a_link_pointing_into_a_protected_directory_is_refused",
                "creating a directory symlink needs Developer Mode or elevation",
            );
            return;
        }
        let through_the_door = door.join("kernel.sys");

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([protected_dir.clone()]))
            .lock_probe(Arc::new(AlwaysUnlocked))
            .build();
        let bin = StubRecycleBin::default();

        assert!(
            brain.protection_reason(&through_the_door).is_none(),
            "the premise: the link's own path is not lexically protected"
        );

        let finding = finding_at(&through_the_door, 8);
        let rec = quarantine_rec(&finding);
        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert!(matches!(
            report.items[0].outcome,
            ExecOutcome::Skipped { .. }
        ));
        assert!(victim.exists(), "the file behind the link must survive");
    }

    /// Security review C2: the execute-time gate accepts only a plain local
    /// drive-letter path. Every other spelling is skipped with a reason, and
    /// the file it names is left exactly where it is.
    ///
    /// This is also the privilege-free companion to the two link/alias tests
    /// above — it needs no reparse point and no 8.3 volume setting, so it
    /// asserts on every machine (security review M11).
    #[test]
    fn a_spelling_the_gate_cannot_judge_is_never_acted_on() {
        let h = harness();
        // An empty list, so nothing here is caught by name — only by shape.
        let brain = brain();
        let bin = StubRecycleBin::default();

        for victim in [
            "\\\\localhost\\C$\\Windows\\System32\\ntoskrnl.exe",
            "\\\\127.0.0.1\\C$\\Windows\\System32\\ntoskrnl.exe",
            "\\\\?\\UNC\\localhost\\c$\\Windows\\System32\\ntoskrnl.exe",
            "\\??\\C:\\Windows\\System32\\ntoskrnl.exe",
            "\\\\.\\C:\\Windows\\System32\\ntoskrnl.exe",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Windows\\System32\\x",
            "C:Windows\\System32\\x",
            "\\Windows\\System32\\x",
            "\\\\server\\share\\x",
        ] {
            let finding = finding_at(Path::new(victim), 4_096);
            let rec = quarantine_rec(&finding);
            let report = execute(
                &[ExecUnit {
                    recommendation: &rec,
                    finding: &finding,
                }],
                &env(&h, &brain, &bin),
                &ExecOptions::default(),
                None,
            )
            .expect("execute");

            match &report.items[0].outcome {
                ExecOutcome::Skipped { reason } => assert!(
                    reason.contains("plain local drive-letter path"),
                    "`{victim}` must say why it was refused, got `{reason}`"
                ),
                other => panic!("`{victim}` must be skipped, got {other:?}"),
            }
            assert_eq!(report.reclaimed_bytes, 0, "`{victim}`");
            assert_eq!(report.batch_id, None, "`{victim}` must move nothing");
        }

        // The control: an ordinary local file in the same batch still moves,
        // so the gate is refusing a shape and not simply refusing everything.
        let ordinary = write(&h.work, "ordinary.tmp", b"movable");
        let finding = finding_at(&ordinary, 7);
        let rec = quarantine_rec(&finding);
        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");
        assert_eq!(report.succeeded(), 1);
        assert!(!ordinary.exists());
    }

    #[test]
    fn info_only_recommendations_do_nothing() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let path = write(&h.work, "informational.txt", b"data");
        let finding = finding_at(&path, 4);
        let rec = Recommendation::new(
            vec![finding.id],
            RiskBucket::DoNotDelete,
            100,
            Action::InfoOnly,
            "inside a protected location",
            0,
        )
        .expect("recommendation");

        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert!(matches!(
            report.items[0].outcome,
            ExecOutcome::Skipped { .. }
        ));
        assert!(path.exists());
    }

    fn recycle_bin_finding(size: u64) -> Finding {
        let mut finding = Finding::new(
            "cleaner",
            Path::new("shell:RecycleBinFolder"),
            FindingKind::RecycleBinItem,
        );
        finding.size_bytes = size;
        finding
    }

    fn confirmed() -> ExecOptions {
        ExecOptions {
            confirm_empty_recycle_bin: true,
        }
    }

    #[test]
    fn the_recycle_bin_is_emptied_rather_than_moved() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::new(12, 5_000);
        let finding = recycle_bin_finding(5_000);
        let rec = quarantine_rec(&finding);

        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &confirmed(),
            None,
        )
        .expect("execute");

        assert_eq!(bin.empty_calls(), 1);
        assert_eq!(report.succeeded(), 1);
        assert_eq!(report.reclaimed_bytes, 5_000);
        assert_eq!(
            report.batch_id, None,
            "emptying the bin moves nothing into quarantine"
        );
    }

    /// H6: the one irreversible action in a batch used to trigger off a bare
    /// id, exactly like a quarantine item.
    #[test]
    fn the_recycle_bin_is_not_emptied_without_its_own_confirmation() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::new(12, 5_000);
        let finding = recycle_bin_finding(5_000);
        let rec = quarantine_rec(&finding);

        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert_eq!(bin.empty_calls(), 0, "the bin must be left alone");
        assert_eq!(report.succeeded(), 0);
        assert_eq!(report.reclaimed_bytes, 0);
        match &report.items[0].outcome {
            ExecOutcome::Skipped { reason } => {
                assert!(reason.contains("confirmation"), "got `{reason}`");
                assert!(reason.contains("quarantine"), "got `{reason}`");
            }
            other => panic!("expected a skip, got {other:?}"),
        }
    }

    /// H6: two recycle-bin units used to empty it twice and count the bytes
    /// twice.
    #[test]
    fn the_recycle_bin_is_emptied_at_most_once_per_batch() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::new(12, 5_000);
        let (first, second) = (recycle_bin_finding(5_000), recycle_bin_finding(5_000));
        let (rec_a, rec_b) = (quarantine_rec(&first), quarantine_rec(&second));

        let report = execute(
            &[
                ExecUnit {
                    recommendation: &rec_a,
                    finding: &first,
                },
                ExecUnit {
                    recommendation: &rec_b,
                    finding: &second,
                },
            ],
            &env(&h, &brain, &bin),
            &confirmed(),
            None,
        )
        .expect("execute");

        assert_eq!(bin.empty_calls(), 1, "once, whatever the batch contains");
        assert_eq!(report.succeeded(), 1);
        assert_eq!(
            report.reclaimed_bytes, 5_000,
            "and the bytes are counted once"
        );
        match &report.items[1].outcome {
            ExecOutcome::Skipped { reason } => {
                assert!(reason.contains("already emptied"), "got `{reason}`");
            }
            other => panic!("expected the second one to be skipped, got {other:?}"),
        }
    }

    /// H6: the reported figure is what emptying actually freed, not what the
    /// scan saw however long ago.
    #[test]
    fn the_reclaimed_bytes_are_measured_at_execute_time() {
        let h = harness();
        let brain = brain();
        // The bin grew since the scan, which said 1 000.
        let bin = StubRecycleBin::new(40, 9_000);
        let finding = recycle_bin_finding(1_000);
        let rec = quarantine_rec(&finding);

        let report = execute(
            &[ExecUnit {
                recommendation: &rec,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &confirmed(),
            None,
        )
        .expect("execute");

        assert_eq!(
            report.reclaimed_bytes, 9_000,
            "the analyze-time figure was stale"
        );
    }

    #[test]
    fn progress_is_reported_for_every_item_plus_a_final_tick() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let paths: Vec<_> = (0..3)
            .map(|i| write(&h.work, &format!("f{i}.tmp"), b"x"))
            .collect();
        let findings: Vec<_> = paths.iter().map(|p| finding_at(p, 1)).collect();
        let recs: Vec<_> = findings.iter().map(quarantine_rec).collect();
        let units: Vec<_> = findings
            .iter()
            .zip(&recs)
            .map(|(finding, recommendation)| ExecUnit {
                recommendation,
                finding,
            })
            .collect();

        let mut ticks = Vec::new();
        let mut sink = |done: u64, total: u64, _: &Finding| ticks.push((done, total));
        execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            Some(&mut sink),
        )
        .expect("execute");

        assert_eq!(ticks, vec![(0, 3), (1, 3), (2, 3), (3, 3)]);
    }

    #[test]
    fn planning_pairs_approved_ids_with_their_findings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = finding_at(&dir.path().join("a"), 1);
        let b = finding_at(&dir.path().join("b"), 2);
        let (ra, rb) = (quarantine_rec(&a), quarantine_rec(&b));
        let recommendations = vec![ra.clone(), rb.clone()];
        let findings = vec![a.clone(), b.clone()];

        let units = plan(&[rb.id], &recommendations, &findings);

        assert_eq!(units.len(), 1);
        assert_eq!(units[0].finding.id, b.id);
    }

    #[test]
    fn planning_ignores_ids_it_does_not_know() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = finding_at(&dir.path().join("a"), 1);
        let recommendations = vec![quarantine_rec(&a)];
        let findings = vec![a];
        let units = plan(&[Uuid::new_v4()], &recommendations, &findings);
        assert!(units.is_empty());
    }

    #[test]
    fn planning_drops_recommendations_whose_finding_vanished() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = finding_at(&dir.path().join("a"), 1);
        let recommendations = vec![quarantine_rec(&a)];
        let approved = vec![recommendations[0].id];
        let units = plan(&approved, &recommendations, &[]);
        assert!(units.is_empty());
    }
}
