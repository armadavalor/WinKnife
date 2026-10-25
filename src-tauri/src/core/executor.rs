//! Turns approved recommendations into a quarantine batch and a report.
//!
//! Why a separate layer: modules decide *what* to suggest, the executor decides
//! *how* it happens, and there is exactly one "how" — quarantine. A module
//! cannot delete anything even if it wanted to, because `execute` hands the
//! executor a list and gets a report back.
//!
//! Four rules this file enforces, in order, for every single item:
//!
//! 1. The recommendation must still validate (no mutating action on a red item).
//! 2. The path must not be protected (re-checked here, not trusted from the
//!    analyze phase — the UI could have been sitting open for an hour).
//! 3. The file must not be in use. Locked means *skipped and reported*, never
//!    failed silently and never a half-finished batch.
//! 4. **The reason must still be true.** Gates 2 and 3 re-check the two things
//!    that can forbid an action; this one re-checks the thing that *justified*
//!    it. See below.
//!
//! # Why the justification is re-checked too (review H10)
//!
//! The first three gates all ask "is this still allowed?". None of them asked
//! "is this still *warranted*?", so between analyze and execute:
//!
//! - a duplicate could be edited into the newest unique copy on the machine and
//!   still be moved under the reason "byte-for-byte identical to `<keeper>`";
//! - the keeper it was compared against could be deleted, leaving the user with
//!   no copy at all;
//! - a browser could be opened, and its cache moved under a rule whose whole
//!   premise (`owner_process_absent`) is that the browser is closed.
//!
//! CLAUDE.md makes the `reason` a hard rule: "Every Recommendation carries a
//! human-readable `reason`." A reason that was true an hour ago and is false now
//! is not a reason, and acting on one is the defect class that erodes the single
//! promise this app makes. So the executor re-derives the preconditions it can
//! and **skips-and-reports** when they no longer hold — never failing the batch,
//! exactly like the lock check above it.
//!
//! What is re-checked is what the finding *recorded*, which is the only thing
//! the executor can verify without becoming a second copy of every module.
//!
//! Every item gets a `tracing` line whatever happens, because this app deletes
//! things and an audit trail is the difference between a bug and a mystery.

use std::collections::HashMap;

use crate::core::db::{with_db, DbAccess};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use uuid::Uuid;

use crate::core::brain::Brain;
use crate::core::error::Result;
use crate::core::platform::{RecycleBin, Registry};
use crate::core::quarantine::Quarantine;
use crate::core::scanner::CancellationToken;
use crate::core::types::{
    Action, ExecItemResult, ExecOutcome, ExecReport, Finding, FindingKind, Justification,
    MoveWarning, Recommendation, StartupSource,
};
use crate::modules::{empty_folders, uninstaller};

/// One unit of work: a recommendation paired with the finding it acts on.
#[derive(Debug, Clone, Copy)]
pub struct ExecUnit<'a> {
    /// The approved recommendation.
    pub recommendation: &'a Recommendation,
    /// The finding it refers to.
    pub finding: &'a Finding,
}

/// An autostart entry as it stands on the machine *right now*.
///
/// The executor's own reading of what `modules::startup` owns — the same
/// arrangement as the TypeScript type mirrors, and for the same reason: `core`
/// must not depend on `modules`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveStartupEntry {
    /// The command line the entry currently runs.
    pub command: String,
    /// Whether it currently runs at all.
    pub enabled: bool,
    /// Whether Windows owns it. These are shown greyed and never toggled
    /// (module spec 05), and gate 4 refuses one that reaches this far.
    pub os_owned: bool,
}

/// What the executor needs from whatever owns autostart entries.
///
/// Small on purpose: gate 4 has to be able to re-read an entry, and the disable
/// itself has to be reversible. Nothing here can delete anything, because there
/// is no method that could.
pub trait StartupControl: Send + Sync {
    /// Read the entry at `source`/`key` as it stands now.
    ///
    /// `conn` is the ledger connection the executor already holds. The control
    /// must use it and never lock the database itself: the executor runs inside
    /// that lock, and the lock is not reentrant (a second lock froze the app on
    /// every disable).
    ///
    /// `Ok(None)` means it is no longer there, which gate 4 treats as a refusal
    /// rather than as success.
    ///
    /// # Errors
    /// Returns an error when the source itself could not be read.
    fn read(
        &self,
        conn: &Connection,
        source: StartupSource,
        key: &str,
    ) -> Result<Option<LiveStartupEntry>>;

    /// Stop the entry running at the next logon, reversibly.
    ///
    /// # Errors
    /// Returns an error when the change was refused (an `HKLM` entry needs
    /// elevation) or the entry is unknown. On success, what the user should
    /// know about a shortcut move that happened only in part.
    fn disable(
        &self,
        conn: &Connection,
        source: StartupSource,
        key: &str,
    ) -> Result<Vec<MoveWarning>>;
}

/// A control that refuses everything, for the batches that contain no autostart
/// entry at all.
///
/// Used by the tests and by any caller that has not composed a startup service:
/// refusing is the safe direction, and a refusal is reported per item rather
/// than failing the batch.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStartupControl;

impl StartupControl for NoStartupControl {
    fn read(
        &self,
        _conn: &Connection,
        _source: StartupSource,
        _key: &str,
    ) -> Result<Option<LiveStartupEntry>> {
        Ok(None)
    }

    fn disable(
        &self,
        _conn: &Connection,
        _source: StartupSource,
        key: &str,
    ) -> Result<Vec<MoveWarning>> {
        Err(crate::core::error::CoreError::UnknownStartupEntry(
            key.to_owned(),
        ))
    }
}

/// The per-user settings catalogue, as the executor sees it (K-018).
///
/// The executor never names a registry path itself: it hands over a catalogue
/// id and the value the scan saw, and the control decides. Every write it makes
/// records the prior value first, so the only way to change a setting here is
/// a way that can be put back.
pub trait TweakControl: Send + Sync {
    /// Why applying `tweak_id` would be wrong right now, or `None` when it is
    /// still what the user reviewed: a known, fenced-in tweak whose live value
    /// is still `observed` and not already the target.
    fn refusal(&self, tweak_id: &str, observed: Option<u32>) -> Option<String>;

    /// Record the prior value in `conn`, write the target, read it back.
    ///
    /// # Errors
    /// Returns an error when the tweak is refused, the write fails, or the
    /// read-back disagrees (the prior value is then put back).
    fn apply(&self, conn: &Connection, tweak_id: &str) -> Result<()>;
}

/// A control that refuses every tweak, for batches built without the tweak
/// service.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTweakControl;

impl TweakControl for NoTweakControl {
    fn refusal(&self, tweak_id: &str, _observed: Option<u32>) -> Option<String> {
        Some(format!("skipped: no tweak service for `{tweak_id}`"))
    }

    fn apply(&self, _conn: &Connection, tweak_id: &str) -> Result<()> {
        Err(crate::core::error::CoreError::Tweak(format!(
            "no tweak service for `{tweak_id}`"
        )))
    }
}

/// Everything the executor is allowed to touch.
pub struct ExecEnvironment<'a> {
    /// Where removed items go.
    pub quarantine: &'a Quarantine,
    /// The ledger. Locked per statement, never for the whole run, so the
    /// rest of the app stays responsive while a clean moves files (#37).
    pub db: &'a dyn DbAccess,
    /// Re-checks protected paths and locks at execute time.
    pub brain: &'a Brain,
    /// The recycle bin, for the one action that is not a quarantine move.
    pub recycle_bin: &'a dyn RecycleBin,
    /// Autostart entries, for [`Action::DisableStartupEntry`].
    pub startup: &'a dyn StartupControl,
    /// Per-user settings, for [`Action::ApplyTweak`].
    pub tweaks: &'a dyn TweakControl,
    /// Read-only: the uninstaller re-reads the Uninstall keys before a move.
    pub registry: &'a dyn Registry,
}

/// Per-request confirmations, separate from the services in
/// [`ExecEnvironment`] because they belong to one click, not to the app.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecOptions<'a> {
    /// Whether the user confirmed emptying the recycle bin *specifically*.
    ///
    /// Everything else in a batch goes to quarantine and can be undone; this
    /// one is final and takes every drive with it. It used to trigger off a
    /// bare `Uuid` in `approved`, exactly like a quarantine item, so nothing on
    /// the Rust side could tell an informed confirmation from a stray id
    /// (security review H6). Without this flag a recycle-bin item is skipped
    /// and says why.
    pub confirm_empty_recycle_bin: bool,
    /// The user's Stop button (issue #39). Checked before each item, never in
    /// the middle of one: an item is either applied whole or not touched, and
    /// everything after the stop is skipped with [`CANCELLED_REASON`].
    pub cancel: Option<&'a CancellationToken>,
}

/// Why an item after a Stop was not applied.
pub const CANCELLED_REASON: &str = "cancelled by the user";

/// What has already happened in this batch.
#[derive(Debug, Default)]
struct BatchState {
    /// Whether anything reached quarantine, so the report only names a batch
    /// that exists.
    moved_anything: bool,
    /// Whether the recycle bin has already been emptied. Two recycle-bin items
    /// in one batch used to empty it twice and count the bytes twice.
    emptied_bin: bool,
    /// What the move of the item being applied wants the user to know,
    /// handed to that item's result and then cleared (follow-up A).
    warnings: Vec<MoveWarning>,
}

/// Progress callback so the UI can show which item is being handled.
pub type ProgressSink<'a> = &'a mut dyn FnMut(u64, u64, &Finding);

/// Apply `units`, isolating every failure to its own item.
///
/// Returns a report with one entry per unit; the batch never aborts early.
///
/// One run at a time is the caller's job: the `execute` command takes
/// [`crate::core::quarantine::Quarantine::begin`] before the database lock
/// (issue #39). This function takes no lock of its own and none of its three
/// gates (K-006) depend on it.
///
/// # Errors
/// Returns an error only if the ledger itself is unusable. Anything
/// item-specific lands in the report.
pub fn execute(
    units: &[ExecUnit<'_>],
    env: &ExecEnvironment<'_>,
    options: &ExecOptions<'_>,
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

        let cancelled = options.cancel.is_some_and(CancellationToken::is_cancelled);
        let (outcome, reclaimed) = if cancelled {
            (
                ExecOutcome::Skipped {
                    reason: CANCELLED_REASON.to_owned(),
                },
                0,
            )
        } else {
            apply_one(unit, env, options, batch, &mut state)
        };
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
            ExecOutcome::Failed { error, .. } => tracing::warn!(
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
            warnings: std::mem::take(&mut state.warnings),
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
    options: &ExecOptions<'_>,
    batch: Uuid,
    state: &mut BatchState,
) -> (ExecOutcome, u64) {
    let skipped = |reason: String| (ExecOutcome::Skipped { reason }, 0);
    let failed = |err: &crate::core::error::CoreError| (ExecOutcome::failed(err), 0);

    // 1. The recommendation still has to be internally valid.
    if let Err(err) = unit.recommendation.validate() {
        return failed(&err);
    }

    // Non-mutating actions are no-ops by definition; report them honestly
    // rather than pretending work happened.
    if !unit.recommendation.action.is_mutating() {
        return skipped(format!(
            "no action to apply for `{:?}`",
            unit.recommendation.action
        ));
    }

    // Disabling an autostart entry is the other mutating action, and it is not a
    // file operation at all: the "path" of a `Run` value is a registry path and
    // the "path" of a task is a slash-separated task name, neither of which
    // gates 2 and 3 can judge (they would refuse both as unjudgeable spellings).
    // What protects an autostart entry is that Windows may own it, and gate 4
    // re-reads exactly that below.
    if unit.recommendation.action == Action::DisableStartupEntry {
        return disable_startup_entry(unit, env, state);
    }

    // A tweak is a registry value too, guarded by its catalogue fence and the
    // re-read in gate 4 rather than by the path gates.
    if unit.recommendation.action == Action::ApplyTweak {
        return apply_tweak(unit, env);
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
    //
    //    `Brain::path_refusal` is the whole of gate 1, and `Brain::classify`
    //    runs the identical call at analyze time, so the rating the user was
    //    shown and the decision taken here cannot disagree (security review H9).
    if let Some(refusal) = env.brain.path_refusal(&unit.finding.path) {
        tracing::warn!(
            path = %unit.finding.path.display(),
            canonical = refusal.canonical.as_ref().map(|p| p.display().to_string()),
            reason = refusal.reason.as_str(),
            "refused to act on a path the hard gates do not clear"
        );
        return skipped(refusal.reason);
    }

    // 3. Locks are re-checked too: a file can be opened between review and
    //    confirm. Skipped, reported, batch continues.
    //    A folder has no lock state of its own (the probe can only say
    //    `Inaccessible`); the owning module's re-check in gate 4 stands in
    //    for it, so only a module that has one may hand a folder through
    //    (empty folders; uninstaller leftovers, K-026).
    if unit.finding.kind == FindingKind::Directory
        && unit.finding.module != empty_folders::MODULE_ID
        && unit.finding.module != uninstaller::MODULE_ID
    {
        return skipped(
            "skipped: only the empty-folders and uninstaller modules can move a whole folder, \
             because only they re-check the folder right before moving it"
                .to_owned(),
        );
    }
    if unit.finding.kind != FindingKind::Directory {
        let lock = env.brain.lock_state(&unit.finding.path);
        if !lock.is_free() {
            return skipped(format!("skipped: {}", lock.reason()));
        }
    }

    // 4. And the reason itself. Gates 2 and 3 ask whether this is still
    //    permitted; this asks whether it is still warranted (review H10).
    if let Some(reason) = justification_refusal(unit, env) {
        tracing::warn!(
            path = %unit.finding.path.display(),
            module = unit.finding.module.as_str(),
            reason = reason.as_str(),
            "refused to act on a recommendation whose justification no longer holds"
        );
        return skipped(reason);
    }

    match env
        .quarantine
        .take_reporting(env.db, batch, &unit.finding.path)
    {
        Ok(taken) => {
            state.moved_anything = true;
            state.warnings = taken.warnings;
            // What may still be at the source was not reclaimed (round-2
            // review).
            let reclaimed = taken
                .item
                .size_bytes
                .saturating_sub(taken.left_behind_bytes);
            (ExecOutcome::Succeeded, reclaimed)
        }
        Err(err) => failed(&err),
    }
}

/* -------------------------------------------------------------------------
 * Gate 4: is the reason still true?
 * ---------------------------------------------------------------------- */

/// Why the recommendation's own reasoning no longer holds, or `None`.
///
/// # What changed in phase 3
///
/// This used to sniff the *finding's* free-form `meta` for shapes it recognised,
/// so a module that recorded nothing fell through to the first three gates and
/// its actions were never re-verified. Every shipped module happened to record
/// something, so nothing reachable from the UI was unchecked — but the guarantee
/// was "modules that record a justification get it verified" rather than "every
/// action is verified" (`docs/DECISIONS-NEEDED.md`, phase 2 review H10).
///
/// The justification is now part of the contract: [`Recommendation::validate`]
/// refuses to build a mutating action carrying
/// [`Justification::Informational`], and gate 1 above runs that validation on
/// every unit. So by the time control reaches here, anything that can still act
/// is carrying a justification this function knows how to re-derive — the
/// `Informational` arm below is unreachable for a mutating action rather than a
/// silent fall-through.
fn justification_refusal(unit: &ExecUnit<'_>, env: &ExecEnvironment<'_>) -> Option<String> {
    match &unit.recommendation.justification {
        // Gate 1 has already refused a mutating action carrying this, so
        // reaching here means the action changes nothing.
        Justification::Informational => None,
        Justification::Rule { pack_id, rule_id } => {
            // The reason for removing a folder is "it holds no files", and
            // that is only true until something is written into it.
            if unit.finding.module == empty_folders::MODULE_ID {
                if let Some(reason) = empty_folders::recheck_refusal(&unit.finding.path) {
                    return Some(reason);
                }
            }
            // A leftover stops being one the moment an app claims it again.
            if unit.finding.module == uninstaller::MODULE_ID {
                if let Some(reason) =
                    uninstaller::recheck_refusal(&unit.finding.path, env.registry, env.brain)
                {
                    return Some(reason);
                }
            }
            rule_refusal(pack_id, rule_id, unit.finding, env.brain)
        }
        Justification::DuplicateOf {
            content_hash,
            keeper,
            keeper_path,
        } => duplicate_refusal(
            content_hash,
            *keeper,
            keeper_path.as_deref(),
            &unit.finding.path,
        ),
        Justification::StartupEntry {
            source,
            key,
            command,
        } => startup_refusal(*source, key, command, env),
        Justification::Tweak { tweak_id, observed } => env.tweaks.refusal(tweak_id, *observed),
    }
}

/// Re-verify a "this is a redundant copy" reason against the disk right now.
///
/// Ordered cheapest-and-most-catastrophic first: losing the last copy is worse
/// than moving a file whose bytes changed, and both are cheaper to detect than
/// a full re-hash.
fn duplicate_refusal(
    content_hash: &str,
    keeper: bool,
    keeper_path: Option<&std::path::Path>,
    path: &std::path::Path,
) -> Option<String> {
    // The keeper is offered as information only, never as an action. If one
    // reaches here the set has been mangled somewhere upstream; refuse rather
    // than reason about it.
    if keeper {
        return Some(
            "skipped: this is the copy the scan suggested keeping, so it is not \
             ours to remove"
                .to_owned(),
        );
    }

    let Some(keeper_path) = keeper_path else {
        return Some(
            "skipped: the scan did not record which copy this one was compared \
             against, so that it is still there cannot be confirmed"
                .to_owned(),
        );
    };

    // Through `path_compare`: the keeper and this copy could be two spellings of
    // one file, and moving "the duplicate" would then move the only copy.
    if crate::core::path_compare::same_path(keeper_path, path) {
        return Some(format!(
            "skipped: `{}` is another spelling of the copy this was going to be \
             compared against, so removing it would leave none",
            path.display()
        ));
    }

    // The keeper vanishing between analyze and execute turns "one of several
    // copies" into "the only copy". `symlink_metadata`, so a dangling link where
    // the keeper used to be is not mistaken for the keeper.
    if std::fs::symlink_metadata(keeper_path).is_err() {
        return Some(format!(
            "skipped: `{}`, the copy this one was identical to, is no longer \
             there — removing this would leave you with none",
            keeper_path.display()
        ));
    }

    // Full re-hash, not a sample and not the partial hash the scan used to
    // narrow candidates. Sampling would re-introduce exactly the false positive
    // `a_crafted_collision_is_not_a_duplicate` exists to forbid, and this is
    // per *approved* item rather than per candidate: the user has already picked
    // these, and the quarantine move is about to read every byte anyway on a
    // cross-volume copy. Reading the file twice is the price of the reason
    // being true.
    match crate::core::content_hash::full_hash(path) {
        Ok(hash) if hash == content_hash => None,
        Ok(_) => Some(format!(
            "skipped: `{}` has changed since the scan, so it is no longer a copy \
             of anything — its current contents exist nowhere else",
            path.display()
        )),
        Err(err) => Some(format!(
            "skipped: `{}` could not be re-read to confirm it is still a duplicate: {err}",
            path.display()
        )),
    }
}

/// Re-verify a rule-pack reason against the machine and the file right now.
///
/// The path globs are deliberately *not* re-tested. They match on the spelling
/// the rule was written in, the walk may have re-anchored them
/// (`modules::cleaner::GroupRule`), and where the file *is* has not changed —
/// that question belongs to gate 2, which asks it properly. What can have
/// changed is the machine's state and the file's own metadata, which is what
/// `match` is about.
fn rule_refusal(pack_id: &str, rule_id: &str, finding: &Finding, brain: &Brain) -> Option<String> {
    let Some(rule) = brain.rule(pack_id, rule_id) else {
        return Some(format!(
            "skipped: the rule `{pack_id}/{rule_id}` that suggested this is no \
             longer loaded, so its reasoning cannot be confirmed"
        ));
    };

    // Uncached: the scan's 2-second memo answered the question the scan asked,
    // and the user has had as long as they liked since. Module spec 02 says a
    // browser cache is Safe *only* while the browser is closed; until this
    // check, that governed the rating and not the action.
    if brain.owner_process_running_uncached(rule) {
        let process = rule
            .spec
            .match_
            .owner_process_absent
            .as_deref()
            .unwrap_or("the program that owns this data");
        return Some(format!(
            "skipped: {process} is running now, and this was only suggested \
             because it was closed"
        ));
    }

    let criteria = &rule.spec.match_;
    if criteria.min_age_hours.is_none()
        && criteria.min_size_bytes.is_none()
        && criteria.extensions.is_none()
    {
        return None;
    }

    // Fresh metadata, not the figures the scan recorded — that is the whole
    // point. `metadata` reads the directory entry and opens nothing, so this
    // stays as read-only as the scan is meant to be.
    let metadata = match std::fs::metadata(&finding.path) {
        Ok(metadata) => metadata,
        Err(err) => {
            return Some(format!(
                "skipped: `{}` could not be re-checked against its rule: {err}",
                finding.path.display()
            ))
        }
    };

    if let Some(min_age) = criteria.min_age_hours {
        // Mirrors `WalkEntry::age_hours`: measured from the modification time,
        // and a file whose clock cannot be read counts as new.
        let age_hours = metadata
            .modified()
            .ok()
            .map(|modified| (Utc::now() - DateTime::<Utc>::from(modified)).num_hours())
            .unwrap_or(0);
        if age_hours < i64::from(min_age) {
            return Some(format!(
                "skipped: `{}` was changed {age_hours}h ago, and this rule only \
                 covers files untouched for {min_age}h",
                finding.path.display()
            ));
        }
    }

    if let Some(min_size) = criteria.min_size_bytes {
        if metadata.len() < min_size {
            return Some(format!(
                "skipped: `{}` is now {} bytes, below the {min_size} this rule covers",
                finding.path.display(),
                metadata.len()
            ));
        }
    }

    if let Some(extensions) = &criteria.extensions {
        let actual = finding
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());
        let allowed = actual.as_deref().is_some_and(|actual| {
            extensions
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(actual))
        });
        if !allowed {
            return Some(format!(
                "skipped: `{}` is not one of the file types this rule covers",
                finding.path.display()
            ));
        }
    }

    None
}

/// Turn an autostart entry off, reversibly.
///
/// Gate 4 runs first and does all the checking: it re-reads the entry from its
/// source and refuses when it has gone, when Windows owns it, or when it now
/// runs a different program than the one the user was shown. Only then is the
/// flag flipped.
fn disable_startup_entry(
    unit: &ExecUnit<'_>,
    env: &ExecEnvironment<'_>,
    state: &mut BatchState,
) -> (ExecOutcome, u64) {
    if let Some(reason) = justification_refusal(unit, env) {
        tracing::warn!(
            entry = %unit.finding.path.display(),
            reason = reason.as_str(),
            "refused to disable an autostart entry whose justification no longer holds"
        );
        return (ExecOutcome::Skipped { reason }, 0);
    }

    let Justification::StartupEntry { source, key, .. } = &unit.recommendation.justification else {
        // Unreachable: `validate` refuses a mutating action with an
        // unverifiable justification, and gate 4 above accepted this one.
        return (
            ExecOutcome::Skipped {
                reason: "skipped: this is not an autostart entry".to_owned(),
            },
            0,
        );
    };

    match with_db(env.db, |conn| env.startup.disable(conn, *source, key)) {
        // No bytes are reclaimed by turning something off, and saying otherwise
        // would put an invented figure in the report.
        // A shortcut moved only in part says so in the item's result
        // (round-2 review).
        Ok(warnings) => {
            state.warnings = warnings;
            (ExecOutcome::Succeeded, 0)
        }
        Err(err) => (ExecOutcome::failed(&err), 0),
    }
}

fn apply_tweak(unit: &ExecUnit<'_>, env: &ExecEnvironment<'_>) -> (ExecOutcome, u64) {
    if let Some(reason) = justification_refusal(unit, env) {
        tracing::warn!(
            setting = %unit.finding.path.display(),
            reason = reason.as_str(),
            "refused a tweak whose justification no longer holds"
        );
        return (ExecOutcome::Skipped { reason }, 0);
    }

    let Justification::Tweak { tweak_id, .. } = &unit.recommendation.justification else {
        return (
            ExecOutcome::Skipped {
                reason: "skipped: this is not a tweak".to_owned(),
            },
            0,
        );
    };

    match with_db(env.db, |conn| env.tweaks.apply(conn, tweak_id)) {
        Ok(()) => (ExecOutcome::Succeeded, 0),
        Err(err) => (ExecOutcome::failed(&err), 0),
    }
}

/// Re-verify an autostart reason against the machine right now.
fn startup_refusal(
    source: StartupSource,
    key: &str,
    command: &str,
    env: &ExecEnvironment<'_>,
) -> Option<String> {
    let live = match with_db(env.db, |conn| env.startup.read(conn, source, key)) {
        Ok(Some(live)) => live,
        Ok(None) => {
            return Some(format!(
                "skipped: `{key}` is no longer in this autostart location, so there \
                 is nothing left to turn off"
            ))
        }
        Err(err) => {
            return Some(format!(
                "skipped: `{key}` could not be re-read before changing it: {err}"
            ))
        }
    };

    // Module spec 05: OS entries are shown greyed with the toggle disabled. That
    // is a UI statement; this is the enforcement, and it runs whatever the UI
    // sent.
    if live.os_owned {
        return Some(format!(
            "skipped: `{key}` is part of Windows, and WinKnife does not turn off \
             the operating system's own startup entries"
        ));
    }

    if !live.enabled {
        return Some(format!("skipped: `{key}` is already disabled"));
    }

    // The premise of the whole row — "this program runs at logon" — names a
    // specific program. If the entry now runs a different one, the signature
    // check, the impact tier and the known-app verdict the user was shown were
    // all about something else.
    if live.command != command {
        return Some(format!(
            "skipped: `{key}` now runs a different program than the one this was \
             about (`{}` rather than `{command}`)",
            live.command
        ));
    }

    None
}

/// Empty the recycle bin, at most once per batch and only when the user said so
/// about this specific action.
fn empty_recycle_bin(
    unit: &ExecUnit<'_>,
    env: &ExecEnvironment<'_>,
    options: &ExecOptions<'_>,
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
        Err(err) => (ExecOutcome::failed(&err), 0),
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
    use crate::core::platform::{
        AlwaysUnlocked, LockProbe, LockState, MapRegistry, StubRecycleBin,
    };
    use crate::core::quarantine::RetentionPolicy;
    use crate::core::scanner::CancellationToken;
    use crate::core::types::RiskBucket;
    use crate::modules::duplicates::{self, DuplicateContext, DuplicateScan, DuplicatesSettings};
    use rusqlite::Connection;
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

    /// Backdate a file's modification time, so a rule's `min_age_hours` can be
    /// satisfied without the test waiting a day.
    fn backdate(path: &Path, hours: u64) {
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(hours * 3600);
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(when)
                    .set_accessed(when),
            )
            .expect("backdate");
    }

    fn finding_at(path: &Path, size: u64) -> Finding {
        let mut finding = Finding::new("cleaner", path, FindingKind::File);
        finding.size_bytes = size;
        finding
    }

    /// A pack whose single rule declares no `match` criteria at all, so gate 4
    /// finds it loaded and has nothing left to re-check.
    ///
    /// Every brain these tests build loads it. That is not a convenience: since
    /// phase 3 a mutating action *cannot be constructed* without a justification
    /// gate 4 can re-derive, so a test that wants to reach the quarantine has to
    /// supply one that resolves — which is exactly the property being asserted.
    fn fixture_pack() -> crate::core::brain::RulePack {
        const JSON: &str = r#"{
            "id": "executor-fixture",
            "name": "Executor fixture",
            "version": 1,
            "module": "cleaner",
            "enabled_by_default": true,
            "description": "One rule with no criteria, for the executor's own tests.",
            "rules": [
                {
                    "id": "anything",
                    "paths": ["%FIXTURE%\\**"],
                    "score": { "base": 95, "max_bucket": "safe" },
                    "action": "quarantine",
                    "reason": "A fixture rule that asserts nothing about the file."
                }
            ]
        }"#;
        crate::core::brain::rules::compile_pack(
            "executor-fixture",
            JSON,
            &crate::core::platform::MapEnv::new()
                .with("USERPROFILE", "C:\\WinKnifeFixture")
                .with("FIXTURE", "C:\\WinKnifeFixture"),
        )
        .expect("the executor fixture pack")
    }

    /// The justification the fixture pack answers for.
    fn a_rule() -> Justification {
        Justification::Rule {
            pack_id: "executor-fixture".to_owned(),
            rule_id: "anything".to_owned(),
        }
    }

    fn quarantine_rec(finding: &Finding) -> Recommendation {
        Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            95,
            Action::Quarantine,
            "temporary file older than 24h",
            finding.size_bytes,
            a_rule(),
        )
        .expect("valid recommendation")
    }

    fn brain() -> Brain {
        Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .packs(vec![fixture_pack()])
            .lock_probe(Arc::new(AlwaysUnlocked))
            .build()
    }

    fn env<'a>(h: &'a Harness, brain: &'a Brain, bin: &'a StubRecycleBin) -> ExecEnvironment<'a> {
        ExecEnvironment {
            quarantine: &h.quarantine,
            db: &h.conn,
            brain,
            recycle_bin: bin,
            startup: &NoStartupControl,
            tweaks: &NoTweakControl,
            registry: Box::leak(Box::new(MapRegistry::new())),
        }
    }

    /// A lock probe that takes its time over one file, standing in for any
    /// slow step of a clean: a large move, a slow disk, an antivirus scan.
    struct SlowProbe {
        entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
        hold: std::time::Duration,
    }

    impl LockProbe for SlowProbe {
        fn probe(&self, _path: &Path) -> LockState {
            if let Some(entered) = self.entered.lock().expect("sender").take() {
                entered.send(()).expect("signal");
                std::thread::sleep(self.hold);
            }
            LockState::Free
        }
    }

    /// Audit issue #37: a clean used to hold the one database connection for
    /// its whole run, so every other command waited behind it. The executor
    /// now locks per statement; a read from another thread must get through
    /// while the clean is still busy with a file.
    #[test]
    fn a_long_clean_does_not_hold_the_database() {
        const HOLD: std::time::Duration = std::time::Duration::from_millis(1500);
        const FAST_ENOUGH: std::time::Duration = std::time::Duration::from_millis(750);

        let h = harness();
        let database = crate::core::db::Database::new(db::open_in_memory().expect("db"));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .packs(vec![fixture_pack()])
            .lock_probe(Arc::new(SlowProbe {
                entered: std::sync::Mutex::new(Some(entered_tx)),
                hold: HOLD,
            }))
            .build();
        let bin = StubRecycleBin::default();
        let first = write(&h.work, "first.tmp", b"one");
        let second = write(&h.work, "second.tmp", b"two");
        let (f1, f2) = (finding_at(&first, 3), finding_at(&second, 3));
        let (r1, r2) = (quarantine_rec(&f1), quarantine_rec(&f2));
        let units = vec![
            ExecUnit {
                recommendation: &r1,
                finding: &f1,
            },
            ExecUnit {
                recommendation: &r2,
                finding: &f2,
            },
        ];
        let (report, waited) = std::thread::scope(|scope| {
            let clean = scope.spawn(|| {
                let env = ExecEnvironment {
                    quarantine: &h.quarantine,
                    db: &database,
                    brain: &brain,
                    recycle_bin: &bin,
                    startup: &NoStartupControl,
                    tweaks: &NoTweakControl,
                    registry: &MapRegistry::new(),
                };
                execute(&units, &env, &ExecOptions::default(), None)
            });
            entered_rx.recv().expect("the clean reached its slow step");
            let asked = std::time::Instant::now();
            database
                .with(|conn| h.quarantine.stats(conn))
                .expect("a read during the clean");
            let waited = asked.elapsed();
            (
                clean.join().expect("clean thread").expect("execute"),
                waited,
            )
        });

        assert!(
            waited < FAST_ENOUGH,
            "a read waited {waited:?} behind a clean that was busy for {HOLD:?}"
        );
        assert_eq!(report.succeeded(), 2);
        let held = database
            .with(|conn| h.quarantine.stats(conn))
            .expect("stats");
        assert_eq!(held.item_count, 2, "the ledger still records both moves");
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

    /// Round-2 review: a take whose source may still be there succeeds (its
    /// copy is held) but reclaims nothing, and says why.
    #[test]
    fn a_take_that_left_its_source_reclaims_nothing_and_says_so() {
        use crate::core::quarantine::seams;
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let file = write(&h.work, "kept.tmp", b"payload");
        let finding = finding_at(&file, 7);
        let rec = quarantine_rec(&finding);
        let units = vec![ExecUnit {
            recommendation: &rec,
            finding: &finding,
        }];

        let _cross = seams::force_cross_volume();
        let _fail = seams::fail_delete_at(0);
        let _unreadable = seams::fail_identity_read();
        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert_eq!(report.succeeded(), 1, "the copy is held: {report:?}");
        assert_eq!(report.reclaimed_bytes, 0, "the source may still be there");
        assert_eq!(report.items[0].reclaimed_bytes, 0);
        assert_eq!(report.items[0].warnings.len(), 1, "{report:?}");
        assert_eq!(report.items[0].warnings[0].path, file.display().to_string());
        assert!(file.exists());
    }

    /// A startup control whose disable succeeds with one warning.
    struct WarningStartup;

    impl StartupControl for WarningStartup {
        fn read(
            &self,
            _conn: &Connection,
            _source: StartupSource,
            _key: &str,
        ) -> Result<Option<LiveStartupEntry>> {
            Ok(Some(LiveStartupEntry {
                command: r"C:\Vendor\vendor.exe".to_owned(),
                enabled: true,
                os_owned: false,
            }))
        }

        fn disable(
            &self,
            _conn: &Connection,
            _source: StartupSource,
            _key: &str,
        ) -> Result<Vec<MoveWarning>> {
            Ok(vec![MoveWarning {
                path: r"C:\Startup\Vendor.lnk".to_owned(),
                reason: "both kept".to_owned(),
            }])
        }
    }

    /// Round-2 review, MEDIUM: a startup disable's move warnings reach the
    /// item's result instead of being dropped.
    #[test]
    fn a_startup_disable_hands_its_warnings_to_the_report() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let finding = Finding::new(
            "startup",
            PathBuf::from(r"C:\Vendor\vendor.exe"),
            FindingKind::File,
        );
        let rec = Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            80,
            Action::DisableStartupEntry,
            "a known updater that does not need to run at logon",
            0,
            Justification::StartupEntry {
                source: StartupSource::UserStartupFolder,
                key: "Vendor.lnk".to_owned(),
                command: r"C:\Vendor\vendor.exe".to_owned(),
            },
        )
        .expect("valid recommendation");
        let units = vec![ExecUnit {
            recommendation: &rec,
            finding: &finding,
        }];
        let env = ExecEnvironment {
            startup: &WarningStartup,
            ..env(&h, &brain, &bin)
        };

        let report = execute(&units, &env, &ExecOptions::default(), None).expect("execute");

        assert_eq!(report.succeeded(), 1, "{report:?}");
        assert_eq!(
            report.items[0].warnings,
            vec![MoveWarning {
                path: r"C:\Startup\Vendor.lnk".to_owned(),
                reason: "both kept".to_owned(),
            }]
        );
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
            .packs(vec![fixture_pack()])
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
            .packs(vec![fixture_pack()])
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
            .packs(vec![fixture_pack()])
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
            a_rule(),
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
            .packs(vec![fixture_pack()])
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
            .packs(vec![fixture_pack()])
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

    /// The same refusal through a **junction**, which needs no privilege — so
    /// this asserts on every developer machine rather than only in CI
    /// (`docs/DECISIONS-NEEDED.md`, architect resolution before phase 2).
    #[cfg(windows)]
    #[test]
    fn a_junction_pointing_into_a_protected_directory_is_refused() {
        let h = harness();
        let protected_dir = h.work.join("Windows");
        std::fs::create_dir_all(&protected_dir).expect("dir");
        let victim = write(&protected_dir, "kernel.sys", b"critical");

        let door = h.work.join("innocent-junction");
        assert!(
            crate::core::test_support::make_junction(&door, &protected_dir),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );
        let through_the_door = door.join("kernel.sys");

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([protected_dir.clone()]))
            .packs(vec![fixture_pack()])
            .lock_probe(Arc::new(AlwaysUnlocked))
            .build();
        let bin = StubRecycleBin::default();

        assert!(
            brain.protection_reason(&through_the_door).is_none(),
            "the premise: the junction's own path is not lexically protected"
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

        assert!(
            matches!(report.items[0].outcome, ExecOutcome::Skipped { .. }),
            "got {:?}",
            report.items[0].outcome
        );
        assert!(victim.exists(), "the file behind the junction must survive");
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
        let rec = Recommendation::informational(
            vec![finding.id],
            RiskBucket::DoNotDelete,
            100,
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

    fn confirmed() -> ExecOptions<'static> {
        ExecOptions {
            confirm_empty_recycle_bin: true,
            ..Default::default()
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

    /* ---------------------------------------------------------------------
     * Stop button (issue #39)
     * ------------------------------------------------------------------ */

    fn cancelled_reasons(report: &ExecReport) -> usize {
        report
            .items
            .iter()
            .filter(|item| {
                matches!(&item.outcome, ExecOutcome::Skipped { reason } if reason == CANCELLED_REASON)
            })
            .count()
    }

    #[test]
    fn a_batch_stopped_before_it_starts_moves_nothing() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let paths: Vec<_> = (0..3)
            .map(|i| write(&h.work, &format!("s{i}.tmp"), b"x"))
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
        let token = CancellationToken::new();
        token.cancel();

        let mut ticks = Vec::new();
        let mut sink = |done: u64, total: u64, _: &Finding| ticks.push((done, total));
        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions {
                cancel: Some(&token),
                ..Default::default()
            },
            Some(&mut sink),
        )
        .expect("execute");

        assert_eq!(report.succeeded(), 0);
        assert_eq!(cancelled_reasons(&report), 3);
        assert!(report.batch_id.is_none(), "nothing moved, so no batch");
        assert!(paths.iter().all(|p| p.exists()));
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 0);
        assert_eq!(ticks.last(), Some(&(3, 3)), "the final tick still goes out");
    }

    #[test]
    fn a_stop_mid_batch_keeps_what_moved_and_skips_the_rest() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let paths: Vec<_> = (0..3)
            .map(|i| write(&h.work, &format!("m{i}.tmp"), b"x"))
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
        let token = CancellationToken::new();
        // The user presses Stop while item 1 is about to start.
        let mut sink = |done: u64, _: u64, _: &Finding| {
            if done == 1 {
                token.cancel();
            }
        };
        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions {
                cancel: Some(&token),
                ..Default::default()
            },
            Some(&mut sink),
        )
        .expect("execute");

        assert_eq!(report.succeeded(), 1);
        assert_eq!(cancelled_reasons(&report), 2);
        assert!(
            report.batch_id.is_some(),
            "item 0 is in quarantine and can be undone"
        );
        assert!(!paths[0].exists());
        assert!(paths[1].exists() && paths[2].exists());
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 1);
    }

    /* ---------------------------------------------------------------------
     * Gate 4: the reason has to still be true (review H10)
     * ------------------------------------------------------------------ */

    /// Run the real duplicate pipeline over `root` and hand back everything the
    /// executor needs, so these tests exercise the same path the app does rather
    /// than a hand-built recommendation that could disagree with it.
    fn duplicate_scan(root: &Path, brain: &Brain) -> (DuplicateScan, Vec<Recommendation>) {
        let protected = ProtectedPaths::empty();
        let settings = DuplicatesSettings {
            min_size_bytes: duplicates::MIN_ALLOWED_SIZE_BYTES,
            roots: Vec::new(),
        };
        let cancel = CancellationToken::new();
        let scan = duplicates::scan(
            &DuplicateContext {
                roots: std::slice::from_ref(&root.to_path_buf()),
                protected: &protected,
                settings: &settings,
                cancel: &cancel,
            },
            None,
        );
        let recommendations = duplicates::analyze(&scan, brain);
        (scan, recommendations)
    }

    /// The ids of every recommendation that actually proposes a move — i.e. what
    /// a user clicking "clean everything offered" would approve.
    fn approve_the_quarantines(recommendations: &[Recommendation]) -> Vec<Uuid> {
        recommendations
            .iter()
            .filter(|rec| rec.action == Action::Quarantine)
            .map(|rec| rec.id)
            .collect()
    }

    fn outcome_for<'a>(report: &'a ExecReport, path: &Path) -> &'a ExecOutcome {
        &report
            .items
            .iter()
            .find(|item| item.path == path)
            .unwrap_or_else(|| panic!("`{}` must be reported", path.display()))
            .outcome
    }

    /// Print the two lines the review reported this defect with, so a run of
    /// these tests is the evidence rather than a claim about it.
    fn report_evidence(report: &ExecReport, path: &Path) {
        println!(
            "outcome for {}: {:?}",
            path.display(),
            outcome_for(report, path)
        );
        println!("file still on disk after execute? {}", path.exists());
    }

    /// **H10(a), reproduced.** Scan a duplicate set, rewrite the non-keeper with
    /// different bytes, then execute. Before this gate the file was moved out
    /// under the reason "Byte-for-byte identical to `<keeper>`" — false at the
    /// moment of the move, and the bytes destroyed were the newest unique copy
    /// on the machine.
    #[test]
    fn a_duplicate_edited_after_the_scan_is_skipped_and_says_why() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();

        let shared = vec![b'd'; 64 * 1024];
        let keeper = write(&h.work, "original.bin", &shared);
        let copy = write(&h.work, "original - Copy.bin", &shared);

        let (scan, recommendations) = duplicate_scan(&h.work, &brain);
        assert_eq!(scan.sets.len(), 1, "the fixture must form one set");
        let approved = approve_the_quarantines(&recommendations);
        assert_eq!(approved.len(), 1, "one keeper, one copy to offer");

        // The user edits the copy while the review screen is open. It is now the
        // only file on the machine with these contents.
        let unique = vec![b'z'; 64 * 1024];
        std::fs::write(&copy, &unique).expect("rewrite the copy");

        let units = plan(&approved, &recommendations, &scan.findings);
        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        let moved = units[0].finding.path.clone();
        report_evidence(&report, &moved);
        match outcome_for(&report, &moved) {
            ExecOutcome::Skipped { reason } => {
                assert!(
                    reason.contains("changed since the scan"),
                    "the skip must name the precondition that failed: `{reason}`"
                );
            }
            other => panic!("expected a skip, got {other:?}"),
        }

        assert!(moved.exists(), "the user's newest unique file must survive");
        assert_eq!(
            std::fs::read(&moved).expect("read"),
            unique,
            "and survive unmodified"
        );
        assert!(keeper.exists());
        assert_eq!(report.succeeded(), 0);
        assert_eq!(report.reclaimed_bytes, 0);
        assert_eq!(report.batch_id, None, "nothing moved, so no batch");
    }

    /// The control for the test above: the identical pipeline with nothing
    /// touched still moves the copy. Without this, "skipped" could just mean the
    /// gate refuses everything.
    #[test]
    fn an_unchanged_duplicate_still_moves() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();

        let shared = vec![b'd'; 64 * 1024];
        write(&h.work, "original.bin", &shared);
        write(&h.work, "original - Copy.bin", &shared);

        let (scan, recommendations) = duplicate_scan(&h.work, &brain);
        let approved = approve_the_quarantines(&recommendations);
        let units = plan(&approved, &recommendations, &scan.findings);
        let moved = units[0].finding.path.clone();

        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert_eq!(
            report.succeeded(),
            1,
            "an untouched duplicate is still a duplicate: {:?}",
            report.items
        );
        assert!(!moved.exists());
        assert_eq!(report.reclaimed_bytes, 64 * 1024);
    }

    /// **The keeper-vanished case.** If the copy this one was declared identical
    /// to is gone by the time the user clicks clean, quarantining "the
    /// duplicate" leaves them with nothing at all.
    #[test]
    fn a_duplicate_whose_keeper_vanished_is_skipped_and_says_why() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();

        let shared = vec![b'k'; 64 * 1024];
        write(&h.work, "original.bin", &shared);
        write(&h.work, "original - Copy.bin", &shared);

        let (scan, recommendations) = duplicate_scan(&h.work, &brain);
        let approved = approve_the_quarantines(&recommendations);
        let units = plan(&approved, &recommendations, &scan.findings);
        let moved = units[0].finding.path.clone();

        // Whichever copy the heuristic nominated, take it away — by asking the
        // scan rather than by guessing which name won.
        //
        // Taken *through the quarantine*, which is both the only sanctioned way
        // to move user data in this crate (`clippy.toml` disallows `remove_*`
        // and `rename`, tests included) and the likeliest way this actually
        // happens: the user cleans one batch, the keeper goes, and then they
        // approve a second batch built from the same scan.
        let keeper_path = scan.sets[0]
            .copies
            .iter()
            .find(|copy| copy.keeper)
            .map(|copy| copy.path.clone())
            .expect("a set always nominates a keeper");
        let earlier_batch = h.quarantine.start_batch();
        h.quarantine
            .take(&h.conn, earlier_batch, &keeper_path)
            .expect("an earlier batch takes the keeper");
        assert!(!keeper_path.exists(), "the keeper really is gone");

        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        report_evidence(&report, &moved);
        match outcome_for(&report, &moved) {
            ExecOutcome::Skipped { reason } => {
                assert!(
                    reason.contains("is no longer there") && reason.contains("leave you with none"),
                    "the skip must say the keeper is gone: `{reason}`"
                );
            }
            other => panic!("expected a skip, got {other:?}"),
        }

        assert!(
            moved.exists(),
            "the last remaining copy must not be quarantined"
        );
        assert_eq!(report.succeeded(), 0);
    }

    /// **Two recommendations naming each other as keeper.** The folder view and
    /// the file-set view can disagree on which copy to keep; approving both must
    /// still leave one copy, because units run in order and the second one's
    /// keeper is gone by then.
    #[test]
    fn copies_that_name_each_other_as_keeper_never_all_go() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();

        let shared = vec![b'x'; 64 * 1024];
        write(&h.work, "a.bin", &shared);
        write(&h.work, "b.bin", &shared);

        let (scan, recommendations) = duplicate_scan(&h.work, &brain);
        let path_of = |id: Uuid| {
            scan.findings
                .iter()
                .find(|finding| finding.id == id)
                .map(|finding| finding.path.clone())
                .expect("every rec points at a finding")
        };
        let mover = recommendations
            .iter()
            .find(|rec| rec.action == Action::Quarantine)
            .expect("one copy is offered");
        let mover_path = path_of(mover.finding_ids[0]);
        let crossed: Vec<Recommendation> = recommendations
            .iter()
            .map(|rec| match &rec.justification {
                Justification::DuplicateOf {
                    content_hash,
                    keeper: true,
                    ..
                } => Recommendation {
                    action: Action::Quarantine,
                    justification: Justification::DuplicateOf {
                        content_hash: content_hash.clone(),
                        keeper: false,
                        keeper_path: Some(mover_path.clone()),
                    },
                    ..rec.clone()
                },
                _ => rec.clone(),
            })
            .collect();

        let approved = approve_the_quarantines(&crossed);
        assert_eq!(approved.len(), 2, "both copies are approved");
        let units = plan(&approved, &crossed, &scan.findings);
        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert_eq!(report.succeeded(), 1, "exactly one copy moves");
        let survivors = crossed
            .iter()
            .filter(|rec| path_of(rec.finding_ids[0]).exists())
            .count();
        assert_eq!(survivors, 1, "one copy always stays");
    }

    /// A probe whose answer can be changed between the scan and the execute, so
    /// "the user opened Chrome while the review screen was up" is expressible.
    #[derive(Debug, Default)]
    struct Switchable {
        running: std::sync::atomic::AtomicBool,
        uncached_calls: std::sync::atomic::AtomicUsize,
    }

    impl Switchable {
        fn start(&self) {
            self.running
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    impl crate::core::platform::ProcessProbe for Switchable {
        fn is_running(&self, _exe_name: &str) -> bool {
            self.running.load(std::sync::atomic::Ordering::Relaxed)
        }

        fn is_running_uncached(&self, exe_name: &str) -> bool {
            self.uncached_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.is_running(exe_name)
        }
    }

    /// A pack whose one rule is a browser cache: green only while the browser is
    /// closed, exactly as module spec 02 describes and the shipped
    /// `browser-cache` pack is written.
    fn browser_pack(cache_dir: &Path) -> crate::core::brain::RulePack {
        // The location comes from `%CACHE%`, so the pack text itself is fixed.
        const JSON: &str = r#"{
            "id": "browser-cache",
            "name": "Browser Caches",
            "version": 1,
            "module": "cleaner",
            "enabled_by_default": true,
            "description": "Cached page data.",
            "rules": [
                {
                    "id": "chrome-cache",
                    "paths": ["%CACHE%\\**"],
                    "match": { "min_age_hours": 24, "owner_process_absent": "chrome.exe" },
                    "score": { "base": 88, "max_bucket": "safe" },
                    "action": "quarantine",
                    "reason": "Cached page data Chrome will rebuild on demand."
                }
            ]
        }"#;
        crate::core::brain::rules::compile_pack(
            "browser-cache",
            JSON,
            &crate::core::platform::MapEnv::new()
                .with("USERPROFILE", cache_dir.to_string_lossy())
                .with("CACHE", cache_dir.to_string_lossy()),
        )
        .expect("the browser fixture pack")
    }

    fn cleaner_finding(path: &Path, size: u64, pack: &str, rule: &str) -> Finding {
        let mut finding = finding_at(path, size);
        finding.meta = serde_json::json!({
            "packId": pack,
            "packName": "Browser Caches",
            "ruleId": rule,
        });
        finding
    }

    /// **H10(b), reproduced.** `Brain::owner_process_running` was consulted in
    /// `classify_by_rule` and nowhere else, so `owner_process_absent` governed
    /// the *rating* and not the *action*. Scan with Chrome closed (green), open
    /// Chrome, clean — and the cache was quarantined under a reason whose whole
    /// premise is that Chrome is closed.
    #[test]
    fn a_cache_is_skipped_when_its_owning_program_opened_after_the_scan() {
        let h = harness();
        let bin = StubRecycleBin::default();
        let cache = h.work.join("Cache");
        std::fs::create_dir_all(&cache).expect("cache dir");
        let entry = write(&cache, "f_000001", b"cached page data");

        let probe = Arc::new(Switchable::default());
        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .packs(vec![browser_pack(&h.work)])
            .lock_probe(Arc::new(AlwaysUnlocked))
            .process_probe(probe.clone())
            .build();

        let finding = cleaner_finding(&entry, 16, "browser-cache", "chrome-cache");
        let rec = Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            88,
            Action::Quarantine,
            "Cached page data Chrome will rebuild on demand.",
            16,
            Justification::Rule {
                pack_id: "browser-cache".to_owned(),
                rule_id: "chrome-cache".to_owned(),
            },
        )
        .expect("recommendation");

        // The user opens Chrome while the review screen is up.
        probe.start();

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

        report_evidence(&report, &entry);
        match outcome_for(&report, &entry) {
            ExecOutcome::Skipped { reason } => {
                assert!(
                    reason.contains("chrome.exe") && reason.contains("running now"),
                    "the skip must name the process: `{reason}`"
                );
            }
            other => panic!("expected a skip, got {other:?}"),
        }
        assert!(
            entry.exists(),
            "a running browser's cache must be left alone"
        );
        assert!(
            probe
                .uncached_calls
                .load(std::sync::atomic::Ordering::Relaxed)
                > 0,
            "the executor must ask the OS, not the scan's 2s memo"
        );
    }

    /// The control: same rule, same pack, Chrome still closed. The gate must let
    /// the ordinary case through.
    #[test]
    fn a_cache_still_moves_when_its_owning_program_is_closed() {
        let h = harness();
        let bin = StubRecycleBin::default();
        let cache = h.work.join("Cache");
        std::fs::create_dir_all(&cache).expect("cache dir");
        let entry = write(&cache, "f_000001", b"cached page data");
        // Old enough for the rule's `min_age_hours`.
        backdate(&entry, 48);

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .packs(vec![browser_pack(&h.work)])
            .lock_probe(Arc::new(AlwaysUnlocked))
            .process_probe(Arc::new(Switchable::default()))
            .build();

        let finding = cleaner_finding(&entry, 16, "browser-cache", "chrome-cache");
        let rec = Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            88,
            Action::Quarantine,
            "Cached page data Chrome will rebuild on demand.",
            16,
            Justification::Rule {
                pack_id: "browser-cache".to_owned(),
                rule_id: "chrome-cache".to_owned(),
            },
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

        assert_eq!(
            report.succeeded(),
            1,
            "the ordinary case must still work: {:?}",
            report.items
        );
        assert!(!entry.exists());
    }

    /// The rest of `match`: a file rewritten while the review screen was open no
    /// longer satisfies the age its rule asked for, and the reason it was
    /// offered under ("older than 24 hours") is now false.
    #[test]
    fn a_file_touched_after_the_scan_no_longer_meets_its_rules_minimum_age() {
        let h = harness();
        let bin = StubRecycleBin::default();
        let cache = h.work.join("Cache");
        std::fs::create_dir_all(&cache).expect("cache dir");
        let entry = write(&cache, "f_000002", b"cached page data");
        backdate(&entry, 48);

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .packs(vec![browser_pack(&h.work)])
            .lock_probe(Arc::new(AlwaysUnlocked))
            .process_probe(Arc::new(Switchable::default()))
            .build();

        let finding = cleaner_finding(&entry, 16, "browser-cache", "chrome-cache");
        let rec = Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            88,
            Action::Quarantine,
            "Cached page data Chrome will rebuild on demand.",
            16,
            Justification::Rule {
                pack_id: "browser-cache".to_owned(),
                rule_id: "chrome-cache".to_owned(),
            },
        )
        .expect("recommendation");

        // Something writes to it after the scan.
        std::fs::write(&entry, b"freshly written").expect("rewrite");

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

        match outcome_for(&report, &entry) {
            ExecOutcome::Skipped { reason } => {
                assert!(
                    reason.contains("untouched for 24h"),
                    "the skip must name the criterion: `{reason}`"
                );
            }
            other => panic!("expected a skip, got {other:?}"),
        }
        assert!(entry.exists());
    }

    /// A recommendation naming a rule no loaded pack has cannot be re-checked at
    /// all, and "cannot confirm" must never mean "proceed".
    #[test]
    fn a_recommendation_whose_rule_is_gone_is_skipped() {
        let h = harness();
        let brain = brain(); // no packs loaded
        let bin = StubRecycleBin::default();
        let victim = write(&h.work, "orphan.tmp", b"data");

        let finding = cleaner_finding(&victim, 4, "browser-cache", "chrome-cache");
        let rec = Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            95,
            Action::Quarantine,
            "a pack that has since been switched off suggested this",
            4,
            Justification::Rule {
                pack_id: "browser-cache".to_owned(),
                rule_id: "chrome-cache".to_owned(),
            },
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

        match outcome_for(&report, &victim) {
            ExecOutcome::Skipped { reason } => {
                assert!(reason.contains("no longer loaded"), "got `{reason}`")
            }
            other => panic!("expected a skip, got {other:?}"),
        }
        assert!(victim.exists());
    }

    /// **Architect addition A, at the executor.** This test used to assert the
    /// opposite: a finding that recorded nothing was quarantined on the strength
    /// of the first three gates alone, and that gap was pinned as a decision.
    ///
    /// It is now closed structurally. `Recommendation::validate` refuses to build
    /// a mutating action carrying `Justification::Informational`, and gate 1 runs
    /// that same validation on every unit — so a hand-forged one (a struct
    /// literal, or a payload deserialised from an older build) is *failed*, not
    /// quietly acted on.
    #[test]
    fn an_action_with_nothing_for_gate_four_to_check_cannot_be_built_or_executed() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();
        let plain = write(&h.work, "no-justification.tmp", b"data");
        let finding = finding_at(&plain, 4);

        // The constructor refuses outright.
        assert!(Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            95,
            Action::Quarantine,
            "no recorded reasoning at all",
            4,
            Justification::Informational,
        )
        .is_err());

        // And the executor refuses the hand-forged struct the constructor would
        // never have produced, rather than falling through to gates 1-3.
        let forged = Recommendation {
            id: Uuid::new_v4(),
            finding_ids: vec![finding.id],
            risk: RiskBucket::Safe,
            confidence: 95,
            action: Action::Quarantine,
            reason: "no recorded reasoning at all".to_owned(),
            savings_bytes: 4,
            justification: Justification::Informational,
        };

        let report = execute(
            &[ExecUnit {
                recommendation: &forged,
                finding: &finding,
            }],
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        match outcome_for(&report, &plain) {
            ExecOutcome::Failed { error, .. } => assert!(
                error.contains("re-verify"),
                "the failure must name the missing guarantee: `{error}`"
            ),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert!(plain.exists(), "nothing unverifiable may be acted on");
        assert_eq!(report.batch_id, None);
    }

    /// One stale item must not take the rest of the batch with it — the same
    /// guarantee the lock check has carried since phase 1.
    #[test]
    fn a_stale_justification_skips_one_item_and_never_the_batch() {
        let h = harness();
        let brain = brain();
        let bin = StubRecycleBin::default();

        let shared = vec![b'm'; 64 * 1024];
        write(&h.work, "original.bin", &shared);
        write(&h.work, "original - Copy.bin", &shared);
        let bystander = write(&h.work, "unrelated.tmp", b"an ordinary file");

        let (scan, mut recommendations) = duplicate_scan(&h.work, &brain);
        let mut approved = approve_the_quarantines(&recommendations);
        let mut findings = scan.findings.clone();

        let bystander_finding = finding_at(&bystander, 16);
        let bystander_rec = quarantine_rec(&bystander_finding);
        approved.push(bystander_rec.id);
        recommendations.push(bystander_rec);
        findings.push(bystander_finding);

        // Break only the duplicate's justification.
        let duplicate_path = plan(&approved, &recommendations, &findings)
            .iter()
            .map(|unit| unit.finding.path.clone())
            .find(|path| path != &bystander)
            .expect("the duplicate unit");
        std::fs::write(&duplicate_path, vec![b'q'; 64 * 1024]).expect("rewrite");

        let units = plan(&approved, &recommendations, &findings);
        let report = execute(
            &units,
            &env(&h, &brain, &bin),
            &ExecOptions::default(),
            None,
        )
        .expect("execute");

        assert!(matches!(
            outcome_for(&report, &duplicate_path),
            ExecOutcome::Skipped { .. }
        ));
        assert_eq!(
            outcome_for(&report, &bystander),
            &ExecOutcome::Succeeded,
            "the healthy item must still move"
        );
        assert!(duplicate_path.exists());
        assert!(!bystander.exists());
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
