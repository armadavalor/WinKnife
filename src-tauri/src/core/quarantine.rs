//! Quarantine: the only way anything leaves its original place.
//!
//! Why this file exists before any feature does: the promise of this app is
//! "everything reversible". Quarantine is that promise in code. Nothing else in
//! the crate is allowed to call `fs::remove_*` on user data — there is a test
//! (`no_other_module_removes_user_data`) that walks the crate's own source and
//! fails the build if anyone tries (ARCHITECTURE section 6, CLAUDE.md hard
//! rules).
//!
//! Layout: `{app_data}/quarantine/{batch_id}/{item_id}{ext}`, with the original
//! path recorded in the `quarantine` table *before* the move, never after — if
//! the process dies mid-move, the ledger still knows where the file belongs.
//!
//! Move semantics: a rename that never replaces when source and destination
//! share a volume (atomic, instant). Across volumes: pin every source file
//! (locking out writers, renamers and deleters), copy through the pins, verify
//! byte for byte, then delete exactly the pinned files through their handles
//! and remove source directories only once they are empty. Anything that
//! arrived in the meantime stays where it is and is logged. A failed copy or
//! verification removes only what the move created and leaves the original
//! untouched.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};
use crate::core::ledger;
use crate::core::path_compare;
use crate::core::platform::{self, pinned, FileId};
use crate::core::types::MoveWarning;

/// Directory name under the app data dir.
pub const QUARANTINE_DIR_NAME: &str = "quarantine";

/// Default retention window, in days (ARCHITECTURE section 6).
pub const DEFAULT_RETENTION_DAYS: i64 = 7;

/// Default ceiling on total quarantine size, in bytes (4 GB).
pub const DEFAULT_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Shortest retention window we will honour. Zero or negative would make every
/// item expire the moment it arrived.
pub const MIN_RETENTION_DAYS: i64 = 1;

/// Longest retention window we will honour.
pub const MAX_RETENTION_DAYS: i64 = 365;

/// Smallest size cap we will honour (64 MB). A smaller one purges a clean the
/// user has not looked at yet.
pub const MIN_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Buffer used when copying and verifying across volumes.
const COPY_BUFFER_BYTES: usize = 64 * 1024;

/// Suffix appended when a restore would overwrite something.
pub const RESTORE_COLLISION_SUFFIX: &str = " (restored)";

/// How many suffixed names a restore tries before giving up.
const RESTORE_ATTEMPTS: u32 = 1_000;

/// How far apart two "identical" modification times may be. Not every
/// filesystem stores the same precision as the one a file was copied from.
const TIME_TOLERANCE: std::time::Duration = std::time::Duration::from_secs(2);

/// How long items are kept and how much space they may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    /// Days before an item becomes eligible for the automatic purge.
    pub retention_days: i64,
    /// Total bytes the quarantine may hold; oldest items are purged first when
    /// this is exceeded.
    pub max_bytes: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_RETENTION_DAYS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

impl RetentionPolicy {
    /// A policy with both numbers forced into a range that cannot destroy the
    /// quarantine.
    ///
    /// The settings table is a plain key/value store of strings: a stray `0`
    /// in `quarantine.retention_days` used to mean "everything expired the
    /// moment it arrived", and the startup purge would then empty the drawer
    /// before the user ever opened it (security review H7). Anything built
    /// from stored or user-supplied numbers goes through here.
    pub fn validated(retention_days: i64, max_bytes: u64) -> Self {
        let clamped = Self {
            retention_days: retention_days.clamp(MIN_RETENTION_DAYS, MAX_RETENTION_DAYS),
            max_bytes: max_bytes.max(MIN_MAX_BYTES),
        };
        if clamped.retention_days != retention_days || clamped.max_bytes != max_bytes {
            tracing::warn!(
                stored_retention_days = retention_days,
                stored_max_bytes = max_bytes,
                retention_days = clamped.retention_days,
                max_bytes = clamped.max_bytes,
                "quarantine retention policy was out of range and has been clamped"
            );
        }
        clamped
    }
}

/// One row of the quarantine ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineItem {
    /// Item id; also the file name inside the batch directory.
    pub id: Uuid,
    /// Batch this item was moved in.
    pub batch_id: Uuid,
    /// Where it came from, and where `restore` puts it back.
    pub original_path: PathBuf,
    /// Where it lives now.
    pub quarantine_path: PathBuf,
    /// Size at the time of the move.
    pub size_bytes: u64,
    /// When it was moved.
    pub quarantined_at: DateTime<Utc>,
    /// First moment the automatic purge may remove it.
    pub purge_after: DateTime<Utc>,
    /// When it was restored, if it was.
    pub restored_at: Option<DateTime<Utc>>,
    /// When a restore was refused because the item is EFS-encrypted and its
    /// way home crosses drives. A marked item is never removed by the
    /// automatic purge, only by the user (decision b, 2026-10-12).
    #[serde(default)]
    pub restore_refused_at: Option<DateTime<Utc>>,
}

impl QuarantineItem {
    /// Whether the item is still physically held.
    pub fn is_held(&self) -> bool {
        self.restored_at.is_none()
    }
}

/// A batch as the quarantine drawer shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineBatch {
    /// Batch id.
    pub batch_id: Uuid,
    /// Items still held in this batch.
    pub item_count: u64,
    /// Bytes still held in this batch.
    pub size_bytes: u64,
    /// When the batch was created.
    pub quarantined_at: DateTime<Utc>,
    /// When its items become purgeable.
    pub purge_after: DateTime<Utc>,
}

/// Totals across everything still held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineStats {
    /// Items still held.
    pub item_count: u64,
    /// Bytes still held.
    pub total_bytes: u64,
    /// Ledger rows that no longer parse and are therefore not counted above.
    ///
    /// A corrupt row used to become a nil-uuid item dated 1970, which the
    /// drawer then offered to restore and the purge treated as long expired
    /// (security review L4). Skipping it is safe; hiding the fact that it
    /// exists is not.
    pub unreadable_rows: u64,
    /// Items whose encrypted restore was refused (decision b), which only
    /// the user removes. Counted so the drawer can say how many (round 4).
    #[serde(default)]
    pub refused_items: u64,
    /// Bytes held by those items. They are not counted against the size cap
    /// and are never evicted for it (decision b, round 4).
    #[serde(default)]
    pub refused_bytes: u64,
    /// The size cap in force, which applies to the other items' bytes only.
    #[serde(default)]
    pub cap_bytes: u64,
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    /// Item restored.
    pub item_id: Uuid,
    /// Where it ended up — not always `original_path`, see `renamed`.
    pub restored_to: PathBuf,
    /// True when something already occupied the original path and the item was
    /// restored alongside it with a suffix instead of overwriting it.
    pub renamed: bool,
    /// What the move back left the user to know about: part of a folder that
    /// stayed in the quarantine, or a copy that is all that remains
    /// (adversarial review follow-up A). Left out of the JSON when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<MoveWarning>,
}

/// What a take did: the item now held, and what its move left the user to know
/// about (adversarial review follow-up A).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeOutcome {
    /// The item recorded in the ledger.
    pub item: QuarantineItem,
    /// Part of a folder that stayed at the original location, and the like.
    pub warnings: Vec<MoveWarning>,
    /// Bytes of the files that may still be at the original location, so they
    /// are not counted as reclaimed (round-2 review).
    pub left_behind_bytes: u64,
}

/// What a purge removed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeReport {
    /// Items removed for good.
    pub purged_items: u64,
    /// Bytes freed from the quarantine directory.
    pub freed_bytes: u64,
    /// Items whose files could not be removed; their ledger rows are kept so
    /// the next purge tries again.
    pub failed_items: Vec<String>,
    /// Batches this purge took items out of.
    ///
    /// The size cap evicts oldest-first and does not stop at a batch boundary,
    /// so a batch the user is still counting on can quietly lose half of
    /// itself. The alternative — purging whole batches — is worse: it destroys
    /// items still inside their retention window because a sibling was old.
    /// So eviction stays item-level and the report names the batches it
    /// touched, for the drawer to show (security review M5).
    pub affected_batches: Vec<String>,
}

impl PurgeReport {
    /// Whether anything at all happened.
    pub fn is_empty(&self) -> bool {
        self.purged_items == 0 && self.failed_items.is_empty()
    }

    /// Record that `batch` lost an item.
    fn touched(&mut self, batch: Uuid) {
        let batch = batch.to_string();
        if !self.affected_batches.contains(&batch) {
            self.affected_batches.push(batch);
        }
    }

    fn merge(&mut self, other: Self) {
        self.purged_items += other.purged_items;
        self.freed_bytes += other.freed_bytes;
        self.failed_items.extend(other.failed_items);
        for batch in other.affected_batches {
            if !self.affected_batches.contains(&batch) {
                self.affected_batches.push(batch);
            }
        }
    }
}

/// Moves things out of harm's way and puts them back.
#[derive(Debug, Clone)]
pub struct Quarantine {
    root: PathBuf,
    policy: RetentionPolicy,
}

impl Quarantine {
    /// A quarantine rooted at `{app_data}/quarantine`.
    ///
    /// The policy is re-validated here whatever the caller did, because this is
    /// the constructor the app uses and a policy read from settings is not
    /// trusted input (security review H7).
    pub fn new(app_data_dir: &Path, policy: RetentionPolicy) -> Self {
        Self {
            root: app_data_dir.join(QUARANTINE_DIR_NAME),
            policy: RetentionPolicy::validated(policy.retention_days, policy.max_bytes),
        }
    }

    /// A quarantine rooted at an explicit directory. Tests point this at a
    /// tempdir; the app never does.
    pub fn with_root(root: impl Into<PathBuf>, policy: RetentionPolicy) -> Self {
        Self {
            root: root.into(),
            policy,
        }
    }

    /// The quarantine directory. Also a protected path — the brain refuses to
    /// recommend cleaning our own storage.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The retention policy in force.
    pub fn policy(&self) -> RetentionPolicy {
        self.policy
    }

    /// Start a batch. Batches exist so the UI can offer "restore everything
    /// that clean removed" as one button.
    pub fn start_batch(&self) -> Uuid {
        Uuid::new_v4()
    }

    /// Move `path` into `batch`, recording it in the ledger first.
    ///
    /// # Errors
    /// Returns [`CoreError::Quarantine`] if the path does not exist or cannot
    /// be moved, and [`CoreError::VerifyFailed`] if a cross-volume copy did not
    /// match the source (in which case the original is still there).
    pub fn take(&self, conn: &Connection, batch: Uuid, path: &Path) -> Result<QuarantineItem> {
        self.take_reporting(conn, batch, path)
            .map(|taken| taken.item)
    }

    /// [`Self::take`], also returning what the move left the user to know
    /// about, so the executor can show it (adversarial review follow-up A).
    ///
    /// # Errors
    /// As [`Self::take`].
    pub fn take_reporting(
        &self,
        conn: &Connection,
        batch: Uuid,
        path: &Path,
    ) -> Result<TakeOutcome> {
        let metadata = fs::symlink_metadata(path).map_err(|err| {
            CoreError::Quarantine(format!("cannot inspect `{}`: {err}", path.display()))
        })?;
        if metadata.is_symlink() {
            // Moving a link would move the link, not the data, and restoring it
            // could recreate a link pointing somewhere that changed meanwhile.
            return Err(CoreError::Quarantine(format!(
                "`{}` is a symbolic link; links are never quarantined",
                path.display()
            )));
        }

        let size_bytes = if metadata.is_dir() {
            directory_size(path)?
        } else {
            metadata.len()
        };

        let item_id = Uuid::new_v4();
        let batch_dir = self.root.join(batch.to_string());
        fs::create_dir_all(&batch_dir).map_err(|err| {
            CoreError::Quarantine(format!(
                "cannot create the batch directory `{}`: {err}",
                batch_dir.display()
            ))
        })?;

        // Keep the extension so the quarantine folder is browsable by a human
        // who wants to check what we took before restoring it.
        let mut file_name = item_id.to_string();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            file_name.push('.');
            file_name.push_str(ext);
        }
        let destination = batch_dir.join(file_name);
        // Nothing this method writes may land outside the quarantine. The name
        // is built from a uuid and an extension so it cannot escape today, but
        // the cleanup path below deletes what it wrote, and that is not a
        // guarantee to leave implicit (security review H1).
        //
        // Through `path_compare` rather than `Path::starts_with`: the latter
        // compares components as they were typed, so a quarantine root reached
        // by one spelling and a destination built from another would read as an
        // escape — and it counts `..` as an ordinary component, so a genuine
        // escape would read as containment.
        if !path_compare::is_inside(&self.root, &destination) {
            return Err(CoreError::Quarantine(format!(
                "refusing to quarantine `{}` to `{}`, which is outside the quarantine root",
                path.display(),
                destination.display()
            )));
        }

        let now = Utc::now();
        let purge_after = now + Duration::days(self.policy.retention_days);
        let item = QuarantineItem {
            id: item_id,
            batch_id: batch,
            original_path: path.to_path_buf(),
            quarantine_path: destination.clone(),
            size_bytes,
            quarantined_at: now,
            purge_after,
            restored_at: None,
            restore_refused_at: None,
        };

        // Ledger first: if the move succeeds and the process then dies, the row
        // already knows where the file came from. A row without a file is
        // recoverable; a file without a row is not.
        //
        // Through `core::ledger` rather than by hand. This function got the
        // order right and said why in a comment, and the startup manager then
        // read the comment, called this file's move primitive, and wrote the
        // bookkeeping around it backwards anyway (security review H11). The
        // order is a type now, and this is one of its callers.
        let mut write = |tx: &Connection| insert_item(tx, &item);
        let mut undo = |tx: &Connection| {
            tx.execute(
                "DELETE FROM quarantine WHERE id = ?1",
                [item_id.to_string()],
            )?;
            Ok(())
        };
        let moved = ledger::ledgered_move(
            conn,
            "quarantine take",
            ledger::LedgerStep::Claim {
                write: &mut write,
                undo: &mut undo,
            },
            |_| {
                move_path_reporting(path, &destination)
                    .map_err(|failure| failure.into_error(path, &destination))
            },
        )?;

        tracing::info!(
            item = %item_id,
            batch = %batch,
            original = %path.display(),
            size_bytes,
            "quarantined"
        );
        Ok(TakeOutcome {
            item,
            warnings: moved.warnings,
            left_behind_bytes: moved.left_bytes,
        })
    }

    /// Put one item back where it came from.
    ///
    /// On a collision the item is restored alongside the existing file with a
    /// ` (restored)` suffix and [`RestoreOutcome::renamed`] is set — never
    /// overwriting whatever is already there.
    ///
    /// # Errors
    /// Returns [`CoreError::UnknownQuarantineItem`] when the id is not in the
    /// ledger or was already restored, and [`CoreError::Quarantine`] when the
    /// move back fails.
    pub fn restore_item(&self, conn: &Connection, item_id: Uuid) -> Result<RestoreOutcome> {
        let item = self
            .find_item(conn, item_id)?
            .ok_or_else(|| CoreError::UnknownQuarantineItem(item_id.to_string()))?;
        if !item.is_held() {
            return Err(CoreError::UnknownQuarantineItem(format!(
                "{item_id} was already restored"
            )));
        }

        // A `Claim`, not a `Retire`: the row is *marked* rather than dropped, so
        // it still names both the quarantine path and the original afterwards
        // and the file never stops being findable. That is what makes marking
        // before the move safe here — a row that says "restored" while the file
        // is still in the drawer leaks a little disk, while a row that says
        // "held" after the file has gone home is a phantom the drawer offers to
        // restore a second time (security review M2).
        let now = Utc::now();
        let mut write = |tx: &Connection| {
            tx.execute(
                "UPDATE quarantine SET restored_at = ?2 WHERE id = ?1",
                rusqlite::params![item_id.to_string(), timestamp(now)],
            )?;
            Ok(())
        };
        let mut undo = |tx: &Connection| {
            tx.execute(
                "UPDATE quarantine SET restored_at = NULL WHERE id = ?1",
                [item_id.to_string()],
            )?;
            Ok(())
        };
        let mut refused_encrypted = false;
        let moved = ledger::ledgered_move(
            conn,
            "quarantine restore",
            ledger::LedgerStep::Claim {
                write: &mut write,
                undo: &mut undo,
            },
            |_| {
                self.move_out(&item).map_err(|failure| match failure {
                    MoveOutFailure::Encrypted(err) => {
                        refused_encrypted = true;
                        err
                    }
                    MoveOutFailure::Other(err) => err,
                })
            },
        );
        let moved = match moved {
            Ok(moved) => moved,
            Err(err) => {
                // After the ledger put the row back to held: decision b keeps
                // an item whose encrypted restore was refused out of the
                // automatic purge.
                if refused_encrypted {
                    let marked = mark_restore_refused(conn, item_id);
                    return Err(encrypted_restore_refused(&item, &err, marked));
                }
                return Err(err);
            }
        };

        tracing::info!(
            item = %item_id,
            restored_to = %moved.target.display(),
            renamed = moved.renamed,
            "restored from quarantine"
        );
        Ok(RestoreOutcome {
            item_id,
            restored_to: moved.target,
            renamed: moved.renamed,
            warnings: moved.warnings,
        })
    }

    /// Move one item back out, claiming its destination exclusively.
    ///
    /// The claim is the point. Probing with `Path::exists` and then renaming
    /// left a window in which something could appear at the target, and
    /// `fs::rename` maps onto `MoveFileEx` *with* `MOVEFILE_REPLACE_EXISTING`,
    /// so it would then be overwritten. Here the rename itself refuses an
    /// occupied destination and the loop simply tries the next name
    /// (security review H2).
    fn move_out(&self, item: &QuarantineItem) -> std::result::Result<MovedOut, MoveOutFailure> {
        let mut contested = 0u32;

        for attempt in 0..=RESTORE_ATTEMPTS {
            let (candidate, renamed) = restore_candidate(&item.original_path, attempt);
            if is_occupied(&candidate) {
                continue;
            }
            if let Some(parent) = candidate.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    MoveOutFailure::Other(CoreError::Quarantine(format!(
                        "cannot recreate `{}` for the restore: {err}",
                        parent.display()
                    )))
                })?;
            }

            match move_path_reporting(&item.quarantine_path, &candidate) {
                Ok(moved) => {
                    return Ok(MovedOut {
                        target: candidate,
                        renamed,
                        warnings: moved.warnings,
                    })
                }
                // Something claimed the name between the probe and the move.
                Err(MoveFailure::Occupied) => {
                    contested += 1;
                    continue;
                }
                // The restore words the refusal once it knows whether the
                // item could be marked to be kept (round 3).
                Err(MoveFailure::Encrypted(err)) => return Err(MoveOutFailure::Encrypted(err)),
                Err(failure) => {
                    return Err(MoveOutFailure::Other(
                        failure.into_error(&item.quarantine_path, &candidate),
                    ))
                }
            }
        }

        Err(MoveOutFailure::Other(CoreError::Quarantine(format!(
            "could not find a free name to restore `{}` to after {RESTORE_ATTEMPTS} attempts \
             ({contested} were taken while we were looking)",
            item.original_path.display()
        ))))
    }

    /// Restore every item still held in `batch`, one by one.
    ///
    /// A failure on one item does not abort the rest; the error is returned
    /// alongside the successes so the UI can report both.
    ///
    /// # Errors
    /// Returns an error only if the ledger cannot be read at all.
    #[allow(clippy::type_complexity)]
    pub fn restore_batch(
        &self,
        conn: &Connection,
        batch: Uuid,
    ) -> Result<(Vec<RestoreOutcome>, Vec<(Uuid, String)>)> {
        let items = self.items_in_batch(conn, batch)?;
        let mut restored = Vec::new();
        let mut failed = Vec::new();

        for item in items.iter().filter(|i| i.is_held()) {
            match self.restore_item(conn, item.id) {
                Ok(outcome) => restored.push(outcome),
                Err(err) => {
                    tracing::warn!(item = %item.id, error = %err, "restore failed");
                    failed.push((item.id, err.to_string()));
                }
            }
        }
        Ok((restored, failed))
    }

    /// Remove one item whose restore was refused for EFS, because the user
    /// asked to. The automatic purge never removes such an item (decision b,
    /// 2026-10-12), so this is the only way it leaves the quarantine short of
    /// a successful restore. Any other item is refused: it is left to its
    /// retention window, and this does not become a second delete button.
    ///
    /// # Errors
    /// Returns [`CoreError::UnknownQuarantineItem`] when the id is not held,
    /// and [`CoreError::QuarantineItemNotMarked`] when its restore was never
    /// refused.
    pub fn purge_item(&self, conn: &Connection, item_id: Uuid) -> Result<PurgeReport> {
        let item = self
            .find_item(conn, item_id)?
            .filter(QuarantineItem::is_held)
            .ok_or_else(|| CoreError::UnknownQuarantineItem(item_id.to_string()))?;
        if item.restore_refused_at.is_none() {
            return Err(CoreError::QuarantineItemNotMarked(
                item.original_path.display().to_string(),
            ));
        }
        self.destroy(conn, &[item], "removed by the user after a refused restore")
    }

    /// Remove everything whose retention window has expired, then enforce the
    /// size cap. Called at app start and from the drawer's "purge now".
    ///
    /// # Errors
    /// Returns an error only if the ledger cannot be read.
    pub fn purge(&self, conn: &Connection, now: DateTime<Utc>) -> Result<PurgeReport> {
        let mut report = self.purge_expired(conn, now)?;
        report.merge(self.enforce_size_cap(conn)?);
        Ok(report)
    }

    /// Remove items whose `purge_after` has passed. An item whose restore
    /// was refused for EFS is kept however old it is (decision b).
    ///
    /// # Errors
    /// Returns an error only if the ledger cannot be read.
    pub fn purge_expired(&self, conn: &Connection, now: DateTime<Utc>) -> Result<PurgeReport> {
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at, restore_refused_at
             FROM quarantine
             WHERE restored_at IS NULL AND restore_refused_at IS NULL
               AND purge_after <= ?1
             ORDER BY purge_after ASC",
        )?;
        let (expired, unreadable) = collect_items(stmt.query_map([timestamp(now)], row_to_item)?)?;
        drop(stmt);
        if unreadable > 0 {
            tracing::error!(
                rows = unreadable,
                "skipped unparsable ledger rows while purging expired items"
            );
        }

        self.destroy(conn, &expired, "retention window expired")
    }

    /// Purge oldest-first until the unmarked held size is back under the cap.
    ///
    /// Items whose restore was refused for EFS are never evicted (decision b)
    /// and, since round 4, are not counted against the cap either: counting
    /// them left whatever they did not fill for everything else, so a clean
    /// just bigger than that was evicted at the next start. The drawer says
    /// what they hold (`refused_items`, `refused_bytes`).
    ///
    /// # Errors
    /// Returns an error only if the ledger cannot be read.
    pub fn enforce_size_cap(&self, conn: &Connection) -> Result<PurgeReport> {
        let held = self.stats(conn)?;
        let unmarked_bytes = held.total_bytes.saturating_sub(held.refused_bytes);
        if unmarked_bytes <= self.policy.max_bytes {
            return Ok(PurgeReport::default());
        }

        let mut over_by = unmarked_bytes - self.policy.max_bytes;
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at, restore_refused_at
             FROM quarantine
             WHERE restored_at IS NULL AND restore_refused_at IS NULL
             ORDER BY quarantined_at ASC",
        )?;
        let (candidates, _) = collect_items(stmt.query_map([], row_to_item)?)?;
        drop(stmt);

        let mut doomed = Vec::new();
        // `candidates` is a finite snapshot and every arm of the loop consumes
        // one element, so this terminates whatever the sizes are.
        for item in candidates {
            if over_by == 0 {
                break;
            }
            // A zero-byte item cannot reduce `over_by`, so it is *skipped*, not
            // evicted and not a reason to stop. Evicting it would purge the
            // quarantine to reclaim nothing (security review M4); stopping at
            // it disabled the cap outright, because candidates are ordered
            // oldest-first and one empty `.tmp` at the head left a quarantine
            // over its cap for the whole retention window (security review H8).
            if item.size_bytes == 0 {
                tracing::debug!(
                    item = %item.id,
                    "skipping a zero-byte item during the size-cap purge: it frees nothing"
                );
                continue;
            }
            over_by = over_by.saturating_sub(item.size_bytes);
            doomed.push(item);
        }
        if over_by > 0 {
            tracing::warn!(
                still_over_by = over_by,
                cap_bytes = self.policy.max_bytes,
                "the size purge could not bring the unmarked items under the cap: \
                 the ledger probably changed while it ran"
            );
        }

        tracing::warn!(
            unmarked_bytes,
            refused_bytes = held.refused_bytes,
            cap_bytes = self.policy.max_bytes,
            purging = doomed.len(),
            "quarantine over its size cap, purging oldest first"
        );
        self.destroy(conn, &doomed, "quarantine size cap exceeded")
    }

    /// Every item still held, newest first.
    ///
    /// # Errors
    /// Returns an error if the ledger cannot be read.
    pub fn list_items(&self, conn: &Connection) -> Result<Vec<QuarantineItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at, restore_refused_at
             FROM quarantine
             WHERE restored_at IS NULL
             ORDER BY quarantined_at DESC",
        )?;
        let (items, _) = collect_items(stmt.query_map([], row_to_item)?)?;
        Ok(items)
    }

    /// Items of one batch, restored ones included, oldest first.
    ///
    /// # Errors
    /// Returns an error if the ledger cannot be read.
    pub fn items_in_batch(&self, conn: &Connection, batch: Uuid) -> Result<Vec<QuarantineItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at, restore_refused_at
             FROM quarantine
             WHERE batch_id = ?1
             ORDER BY quarantined_at ASC",
        )?;
        let (items, _) = collect_items(stmt.query_map([batch.to_string()], row_to_item)?)?;
        Ok(items)
    }

    /// Batches that still hold something, newest first. This is the drawer.
    ///
    /// # Errors
    /// Returns an error if the ledger cannot be read.
    pub fn list_batches(&self, conn: &Connection) -> Result<Vec<QuarantineBatch>> {
        let mut stmt = conn.prepare(
            "SELECT batch_id, count(*), coalesce(sum(size_bytes), 0),
                    min(quarantined_at), min(purge_after)
             FROM quarantine
             WHERE restored_at IS NULL
             GROUP BY batch_id
             ORDER BY min(quarantined_at) DESC",
        )?;
        let batches = stmt
            .query_map([], |row| {
                let batch_id: String = row.get(0)?;
                let quarantined_at: String = row.get(3)?;
                let purge_after: String = row.get(4)?;
                let (Some(batch_id), Some(quarantined_at), Some(purge_after)) = (
                    parse_uuid(&batch_id),
                    parse_time(&quarantined_at),
                    parse_time(&purge_after),
                ) else {
                    return Ok(None);
                };
                Ok(Some(QuarantineBatch {
                    batch_id,
                    item_count: row.get::<_, i64>(1)?.max(0) as u64,
                    size_bytes: row.get::<_, i64>(2)?.max(0) as u64,
                    quarantined_at,
                    purge_after,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(batches)
    }

    /// Totals across everything still held.
    ///
    /// Counted row by row rather than with a SQL aggregate, so the totals agree
    /// with what [`Quarantine::list_items`] will actually show and so a row
    /// nobody can parse is reported instead of silently inflating the numbers
    /// (security review L4).
    ///
    /// # Errors
    /// Returns an error if the ledger cannot be read.
    pub fn stats(&self, conn: &Connection) -> Result<QuarantineStats> {
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at, restore_refused_at
             FROM quarantine
             WHERE restored_at IS NULL",
        )?;
        let (items, unreadable_rows) = collect_items(stmt.query_map([], row_to_item)?)?;
        Ok(QuarantineStats {
            item_count: items.len() as u64,
            total_bytes: items.iter().map(|item| item.size_bytes).sum(),
            unreadable_rows,
            refused_items: items
                .iter()
                .filter(|item| item.restore_refused_at.is_some())
                .count() as u64,
            refused_bytes: items
                .iter()
                .filter(|item| item.restore_refused_at.is_some())
                .map(|item| item.size_bytes)
                .sum(),
            cap_bytes: self.policy.max_bytes,
        })
    }

    /// Look one item up.
    ///
    /// # Errors
    /// Returns an error if the ledger cannot be read.
    pub fn find_item(&self, conn: &Connection, item_id: Uuid) -> Result<Option<QuarantineItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at, restore_refused_at
             FROM quarantine WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([item_id.to_string()], row_to_item)?;
        match rows.next() {
            Some(row) => Ok(row?),
            None => Ok(None),
        }
    }

    /// Physically remove `items` and forget them.
    ///
    /// This is the one place in the crate allowed to delete user data for good,
    /// and it only ever touches paths inside [`Quarantine::root`] — a file that
    /// somehow points outside is left alone and reported.
    fn destroy(
        &self,
        conn: &Connection,
        items: &[QuarantineItem],
        why: &'static str,
    ) -> Result<PurgeReport> {
        let mut report = PurgeReport::default();

        for item in items {
            // `Path::starts_with` treats `..` as an ordinary component, so
            // `…\quarantine\..\..\Windows\System32` "starts with" the
            // quarantine root and walked straight past this guard. `take`
            // writes uuid-based names so nothing reaches here with a `..` in it
            // today — but this is the one function in the crate allowed to
            // delete user data for good, and that is not a property to leave
            // resting on what some other function happens to write (security
            // review M13).
            //
            // Refused before the containment check rather than resolved by it:
            // a path that climbs can land anywhere, and a ledger row asking to
            // permanently delete one is a bug worth reporting, not normalising.
            if path_compare::has_traversal(&item.quarantine_path) {
                tracing::error!(
                    item = %item.id,
                    path = %item.quarantine_path.display(),
                    "refusing to purge a path that climbs with `..`"
                );
                report.failed_items.push(item.id.to_string());
                continue;
            }
            if !path_compare::is_inside(&self.root, &item.quarantine_path) {
                tracing::error!(
                    item = %item.id,
                    path = %item.quarantine_path.display(),
                    root = %self.root.display(),
                    "refusing to purge a path outside the quarantine root"
                );
                report.failed_items.push(item.id.to_string());
                continue;
            }

            // A file copied in from another volume keeps its read-only flag,
            // and Windows refuses to delete a read-only file. Clearing it here
            // is the only way the cap and the retention window can ever reach
            // one (security review M6).
            clear_readonly(&item.quarantine_path);
            let removed = remove_quarantined(&item.quarantine_path);

            match removed {
                Ok(()) => {
                    conn.execute(
                        "DELETE FROM quarantine WHERE id = ?1",
                        [item.id.to_string()],
                    )?;
                    report.purged_items += 1;
                    report.freed_bytes += item.size_bytes;
                    report.touched(item.batch_id);
                    tracing::info!(
                        item = %item.id,
                        original = %item.original_path.display(),
                        size_bytes = item.size_bytes,
                        why,
                        "purged from quarantine"
                    );
                }
                Err(err) => {
                    tracing::warn!(item = %item.id, error = %err, "purge failed, keeping the row");
                    report.failed_items.push(item.id.to_string());
                }
            }
        }

        // Empty batch directories left behind are noise; they hold no user data
        // by construction (every file inside was just removed).
        self.prune_empty_batch_dirs();
        Ok(report)
    }

    #[allow(clippy::disallowed_methods)]
    fn prune_empty_batch_dirs(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && fs::read_dir(&path).is_ok_and(|mut e| e.next().is_none()) {
                if let Err(err) = fs::remove_dir(&path) {
                    tracing::debug!(
                        path = %path.display(),
                        error = %err,
                        "an empty batch directory could not be pruned"
                    );
                }
            }
        }
    }
}

/// Remove one quarantined path for good.
///
/// The one place in the crate allowed to delete user data, and only ever called
/// with a path the caller has already proved is inside the quarantine root.
#[allow(clippy::disallowed_methods)]
fn remove_quarantined(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        // Already gone (manual cleanup, previous half-finished purge).
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Clear the read-only flag on `path` and, if it is a directory, everything
/// under it. Best effort: the removal itself reports what it could not do.
fn clear_readonly(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_symlink() {
        return;
    }
    if metadata.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                clear_readonly(&entry.path());
            }
        }
        return;
    }
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        // On Windows this clears FILE_ATTRIBUTE_READONLY and nothing else, and
        // the path is always inside our own quarantine root — the caller has
        // already proved that. The lint's Unix world-writable warning does not
        // apply to either the platform or the location.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        if let Err(err) = fs::set_permissions(path, permissions) {
            tracing::debug!(
                path = %path.display(),
                error = %err,
                "could not clear the read-only flag before purging"
            );
        }
    }
}

fn insert_item(conn: &Connection, item: &QuarantineItem) -> Result<()> {
    conn.execute(
        "INSERT INTO quarantine
            (id, batch_id, original_path, quarantine_path, size_bytes,
             quarantined_at, purge_after, restored_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        rusqlite::params![
            item.id.to_string(),
            item.batch_id.to_string(),
            item.original_path.to_string_lossy(),
            item.quarantine_path.to_string_lossy(),
            item.size_bytes as i64,
            timestamp(item.quarantined_at),
            timestamp(item.purge_after),
        ],
    )?;
    Ok(())
}

/// How every timestamp this module writes is spelled.
///
/// The retention query compares them with `<=` *as text*, so they have to sort
/// the way they read: fixed width, fixed precision, always `Z`. Plain
/// `to_rfc3339` gives a variable number of fractional digits and would emit an
/// offset if the value were ever built in a local zone, either of which breaks
/// the ordering (security review L2). The result still parses as RFC 3339, so
/// rows written by the previous format are read back unchanged.
fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<QuarantineItem>> {
    let id: String = row.get(0)?;
    let batch_id: String = row.get(1)?;
    let original_path: String = row.get(2)?;
    let quarantine_path: String = row.get(3)?;
    let quarantined_at: String = row.get(5)?;
    let purge_after: String = row.get(6)?;
    let restored_at: Option<String> = row.get(7)?;
    let restore_refused_at: Option<String> = row.get(8)?;

    let (Some(id), Some(batch_id), Some(quarantined_at), Some(purge_after)) = (
        parse_uuid(&id),
        parse_uuid(&batch_id),
        parse_time(&quarantined_at),
        parse_time(&purge_after),
    ) else {
        return Ok(None);
    };
    let (Some(restored_at), Some(restore_refused_at)) = (
        parse_optional_time(restored_at.as_deref()),
        parse_optional_time(restore_refused_at.as_deref()),
    ) else {
        return Ok(None);
    };

    Ok(Some(QuarantineItem {
        id,
        batch_id,
        original_path: PathBuf::from(original_path),
        quarantine_path: PathBuf::from(quarantine_path),
        size_bytes: row.get::<_, i64>(4)?.max(0) as u64,
        quarantined_at,
        purge_after,
        restored_at,
        restore_refused_at,
    }))
}

/// A nullable timestamp column: `Some(None)` when it is empty, `None` when it
/// holds something that does not parse.
fn parse_optional_time(raw: Option<&str>) -> Option<Option<DateTime<Utc>>> {
    match raw {
        Some(raw) => parse_time(raw).map(Some),
        None => Some(None),
    }
}

/// Drain a ledger query into items plus a count of the rows that no longer
/// parse.
fn collect_items(
    rows: impl Iterator<Item = rusqlite::Result<Option<QuarantineItem>>>,
) -> rusqlite::Result<(Vec<QuarantineItem>, u64)> {
    let mut items = Vec::new();
    let mut unreadable = 0;
    for row in rows {
        match row? {
            Some(item) => items.push(item),
            None => unreadable += 1,
        }
    }
    Ok((items, unreadable))
}

/// Ids and timestamps are written by this module and only ever read back here.
/// A corrupted value used to become a nil uuid dated 1970 — an item the drawer
/// offered to restore and the purge thought was long expired. It is skipped and
/// counted now instead (security review L4).
fn parse_uuid(raw: &str) -> Option<Uuid> {
    match Uuid::parse_str(raw) {
        Ok(value) => Some(value),
        Err(_) => {
            tracing::error!(value = raw, "unparsable uuid in the quarantine ledger");
            None
        }
    }
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    match DateTime::parse_from_rfc3339(raw) {
        Ok(value) => Some(value.with_timezone(&Utc)),
        Err(_) => {
            tracing::error!(value = raw, "unparsable timestamp in the quarantine ledger");
            None
        }
    }
}

/// Whether something is at `path`.
///
/// `Path::exists` answers "no" both for "nothing is there" and for "we are not
/// allowed to look", and restoring over the second one would destroy a file
/// (security review H2). Anything but a clean "not found" counts as occupied.
fn is_occupied(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(err) => err.kind() != std::io::ErrorKind::NotFound,
    }
}

/// The `attempt`-th name a restore may use. Attempt 0 is the original path.
fn restore_candidate(original: &Path, attempt: u32) -> (PathBuf, bool) {
    if attempt == 0 {
        return (original.to_path_buf(), false);
    }

    let parent = original.parent().unwrap_or_else(|| Path::new(""));
    if attempt > RESTORE_ATTEMPTS {
        // Astronomically unlikely; a uuid rather than looping forever.
        return (parent.join(Uuid::new_v4().to_string()), true);
    }

    let mut name = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(RESTORE_COLLISION_SUFFIX);
    if attempt > 1 {
        name.push_str(&format!(" {attempt}"));
    }
    if let Some(ext) = original.extension() {
        name.push('.');
        name.push_str(&ext.to_string_lossy());
    }
    (parent.join(name), true)
}

/// A restore refused because the item is EFS-encrypted and its way home
/// crosses drives: say that it stays held and how to get it back
/// (adversarial review follow-up F).
///
/// `marked` is whether the marker that keeps it from the automatic purge was
/// recorded. Only then does the message promise to keep it; otherwise it says
/// the purge may still remove it and by when to act (round 3).
fn encrypted_restore_refused(item: &QuarantineItem, err: &CoreError, marked: bool) -> CoreError {
    let original = item.original_path.display();
    let held_at = item.quarantine_path.display();
    let how = format!(
        "decrypt it where it is held (`{held_at}`: Properties, Advanced, clear \"Encrypt contents\") \
         and restore it again"
    );
    let text = if marked {
        format!(
            "could not restore `{original}`: {err}. It stays in the quarantine as a held item, \
             kept until you remove it: the automatic purge leaves it alone. To get it back, {how}."
        )
    } else {
        format!(
            "could not restore `{original}`: {err}. It stays in the quarantine as a held item, \
             but it could not be marked to be kept, so the automatic purge may still remove it \
             when its time there ends ({}) or when the quarantine is over its size limit. \
             To get it back, {how} before then.",
            item.purge_after.format("%Y-%m-%d %H:%M UTC")
        )
    };
    CoreError::Quarantine(text)
}

/// Why [`Quarantine::move_out`] did not bring an item back. The encrypted
/// refusal is its own variant so the restore can mark the item without
/// reading the message (decision b).
enum MoveOutFailure {
    Encrypted(CoreError),
    Other(CoreError),
}

/// Record that `item_id`'s restore was refused for EFS, so the automatic
/// purge keeps it. Returns whether the marker was recorded: the refusal is
/// still what the user sees either way, but its message only promises to
/// keep the item when this is true (round 3).
fn mark_restore_refused(conn: &Connection, item_id: Uuid) -> bool {
    let marked = conn.execute(
        "UPDATE quarantine SET restore_refused_at = ?2
         WHERE id = ?1 AND restored_at IS NULL",
        rusqlite::params![item_id.to_string(), timestamp(Utc::now())],
    );
    match marked {
        Ok(1) => true,
        Ok(rows) => {
            tracing::error!(
                item = %item_id,
                rows,
                "a refused encrypted restore matched no held row; the automatic purge may remove the item"
            );
            false
        }
        Err(err) => {
            tracing::error!(
                item = %item_id,
                error = %err,
                "could not record a refused encrypted restore; the automatic purge may remove the item"
            );
            false
        }
    }
}

/// Where [`Quarantine::move_out`] put an item, and what its move reported.
struct MovedOut {
    target: PathBuf,
    renamed: bool,
    warnings: Vec<MoveWarning>,
}

/// Why a move did not happen.
enum MoveFailure {
    /// Something already occupies the destination. The caller picks another
    /// name; it never overwrites.
    Occupied,
    /// Anything else, already worded for the user.
    Failed(CoreError),
    /// The source is EFS-encrypted and the move would cross volumes, which
    /// would store it in the clear. Already worded for the user; typed so a
    /// restore can tell it apart without matching its wording (round-2 review).
    Encrypted(CoreError),
}

impl MoveFailure {
    fn into_error(self, from: &Path, to: &Path) -> CoreError {
        match self {
            Self::Occupied => CoreError::Quarantine(format!(
                "cannot move `{}` to `{}`: something is already there",
                from.display(),
                to.display()
            )),
            Self::Failed(err) | Self::Encrypted(err) => err,
        }
    }
}

/// The rename primitive. A function pointer so a test can force the
/// cross-volume branch without a second physical volume (security review M6).
type Rename = fn(&Path, &Path) -> std::io::Result<()>;

/// Rename when we can, copy-verify-remove when we must. Never replaces.
#[cfg(test)]
fn move_path(from: &Path, to: &Path) -> std::result::Result<(), MoveFailure> {
    move_path_reporting(from, to).map(|_| ())
}

/// [`move_path`], also returning what the user should know about a move that
/// happened only in part (adversarial review follow-up A).
fn move_path_reporting(from: &Path, to: &Path) -> std::result::Result<Moved, MoveFailure> {
    move_path_with_report(from, to, default_rename())
}

/// The rename [`move_path`] uses: the real one, unless a test asked for the
/// cross-volume branch with [`seams::force_cross_volume`] so that `take` can
/// be driven through it on a single volume.
#[cfg(test)]
fn default_rename() -> Rename {
    if seams::cross_volume_forced() {
        seams::forced_cross_volume
    } else {
        rename_no_replace
    }
}

#[cfg(not(test))]
fn default_rename() -> Rename {
    rename_no_replace
}

/// The delete failure a test asked for at the `index`-th pinned source file,
/// if any. With [`seams::fail_delete_after_marking_at`] the delete is really
/// marked first, so the file goes when its pin closes although its delete
/// reported a failure.
#[cfg(test)]
fn injected_delete_failure(index: usize, file: &PinnedFile) -> Option<std::io::Error> {
    #[cfg(windows)]
    if seams::marked_delete_failure(index) {
        return Some(match delete_by_handle(&file.handle) {
            Ok(()) => std::io::Error::other("injected failure after the delete was marked"),
            Err(err) => err,
        });
    }
    #[cfg(not(windows))]
    let _ = file;
    seams::delete_failure(index)
}

#[cfg(not(test))]
fn injected_delete_failure(_index: usize, _file: &PinnedFile) -> Option<std::io::Error> {
    None
}

/// The identity-read failure a test asked for, if any.
#[cfg(test)]
fn injected_identity_failure() -> Option<std::io::Error> {
    seams::identity_failure()
}

#[cfg(not(test))]
fn injected_identity_failure() -> Option<std::io::Error> {
    None
}

/// The failure a test asked for in the read that confirms a take-back.
#[cfg(all(windows, test))]
fn injected_confirmation_failure() -> Option<std::io::Error> {
    seams::revoke_confirmation_failure()
}

#[cfg(all(windows, not(test)))]
fn injected_confirmation_failure() -> Option<std::io::Error> {
    None
}

/// Whether a test asked for taking back a delete to be silently ignored.
#[cfg(all(windows, test))]
fn revoke_skipped() -> bool {
    seams::revoke_skipped()
}

#[cfg(all(windows, not(test)))]
fn revoke_skipped() -> bool {
    false
}

/// Test-only hooks into the cross-volume move, at the moments a race would
/// strike: right after verification, right after a destination directory
/// appears, and while source files are deleted.
///
/// Thread-local, so tests running in parallel never see one another's hooks,
/// and cleared by a [`seams::Reset`] guard, so a test that panics cannot leak
/// a hook into the next test on its thread.
#[cfg(test)]
pub(crate) mod seams {
    use std::cell::{Cell, RefCell};
    use std::path::Path;

    type Hook = Box<dyn FnOnce(&Path)>;

    thread_local! {
        static AFTER_VERIFY: RefCell<Option<Hook>> = const { RefCell::new(None) };
        static AFTER_CREATE_DIR: RefCell<Option<Hook>> = const { RefCell::new(None) };
        static BEFORE_WALK: RefCell<Option<Hook>> = const { RefCell::new(None) };
        static FAIL_DELETE_AT: Cell<Option<usize>> = const { Cell::new(None) };
        static FORCE_CROSS_VOLUME: Cell<bool> = const { Cell::new(false) };
        static SKIP_REVOKE: Cell<bool> = const { Cell::new(false) };
        static MARK_THEN_FAIL_AT: Cell<Option<usize>> = const { Cell::new(None) };
        static FAIL_IDENTITY_READ: Cell<bool> = const { Cell::new(false) };
        static FAIL_REVOKE_CONFIRM: Cell<bool> = const { Cell::new(false) };
    }

    /// Clears every seam on this thread when dropped.
    #[must_use = "the seam is cleared as soon as this guard is dropped"]
    pub(crate) struct Reset;

    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_VERIFY.set(None);
            AFTER_CREATE_DIR.set(None);
            BEFORE_WALK.set(None);
            FAIL_DELETE_AT.set(None);
            FORCE_CROSS_VOLUME.set(false);
            SKIP_REVOKE.set(false);
            MARK_THEN_FAIL_AT.set(None);
            FAIL_IDENTITY_READ.set(false);
            FAIL_REVOKE_CONFIRM.set(false);
        }
    }

    /// Run `hook` once, right after a cross-volume copy has been verified.
    pub(crate) fn on_after_verify(hook: impl FnOnce(&Path) + 'static) -> Reset {
        AFTER_VERIFY.set(Some(Box::new(hook)));
        Reset
    }

    /// Run `hook` once, right after the copy creates a destination directory
    /// and before anything is copied into it.
    pub(crate) fn on_after_create_dir(hook: impl FnOnce(&Path) + 'static) -> Reset {
        AFTER_CREATE_DIR.set(Some(Box::new(hook)));
        Reset
    }

    /// Run `hook` once, after the source has been inspected and before it is
    /// opened and walked: the moment a swap for a link would strike.
    pub(crate) fn on_before_walk(hook: impl FnOnce(&Path) + 'static) -> Reset {
        BEFORE_WALK.set(Some(Box::new(hook)));
        Reset
    }

    /// Make the delete of the `index`-th pinned source file fail.
    pub(crate) fn fail_delete_at(index: usize) -> Reset {
        FAIL_DELETE_AT.set(Some(index));
        Reset
    }

    /// Make [`super::move_path`] take the cross-volume branch.
    pub(crate) fn force_cross_volume() -> Reset {
        FORCE_CROSS_VOLUME.set(true);
        Reset
    }

    /// Make taking back a marked delete do nothing while reporting success,
    /// as a filesystem that ignores the request would: the delete stays
    /// pending and takes effect when the pin closes.
    pub(crate) fn skip_revoke() -> Reset {
        SKIP_REVOKE.set(true);
        Reset
    }

    pub(crate) fn revoke_skipped() -> bool {
        SKIP_REVOKE.get()
    }

    /// Make the delete of the `index`-th pinned source file be marked and
    /// then report a failure, as a delete that took effect but said it did
    /// not would.
    pub(crate) fn fail_delete_after_marking_at(index: usize) -> Reset {
        MARK_THEN_FAIL_AT.set(Some(index));
        Reset
    }

    #[cfg(windows)]
    pub(crate) fn marked_delete_failure(index: usize) -> bool {
        MARK_THEN_FAIL_AT.get() == Some(index)
    }

    /// Make reading a source's identity after a refused delete fail with
    /// something other than "not found".
    pub(crate) fn fail_identity_read() -> Reset {
        FAIL_IDENTITY_READ.set(true);
        Reset
    }

    /// Make the read that confirms a taken-back delete fail, after the
    /// take-back itself went through.
    pub(crate) fn fail_revoke_confirmation() -> Reset {
        FAIL_REVOKE_CONFIRM.set(true);
        Reset
    }

    #[cfg(windows)]
    pub(crate) fn revoke_confirmation_failure() -> Option<std::io::Error> {
        FAIL_REVOKE_CONFIRM.get().then(|| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected confirmation failure",
            )
        })
    }

    pub(crate) fn identity_failure() -> Option<std::io::Error> {
        FAIL_IDENTITY_READ.get().then(|| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected identity failure",
            )
        })
    }

    pub(crate) fn run_after_verify(path: &Path) {
        if let Some(hook) = AFTER_VERIFY.take() {
            hook(path);
        }
    }

    pub(crate) fn run_before_walk(path: &Path) {
        if let Some(hook) = BEFORE_WALK.take() {
            hook(path);
        }
    }

    pub(crate) fn run_after_create_dir(path: &Path) {
        if let Some(hook) = AFTER_CREATE_DIR.take() {
            hook(path);
        }
    }

    /// The failure [`fail_delete_at`] asked for, if `index` is the one.
    pub(crate) fn delete_failure(index: usize) -> Option<std::io::Error> {
        (FAIL_DELETE_AT.get() == Some(index))
            .then(|| std::io::Error::other("injected delete failure"))
    }

    pub(crate) fn cross_volume_forced() -> bool {
        FORCE_CROSS_VOLUME.get()
    }

    /// A rename that always answers "different volumes".
    pub(crate) fn forced_cross_volume(_: &Path, _: &Path) -> std::io::Result<()> {
        #[cfg(windows)]
        const CROSS_VOLUME: i32 = 17;
        #[cfg(not(windows))]
        const CROSS_VOLUME: i32 = 18;
        Err(std::io::Error::from_raw_os_error(CROSS_VOLUME))
    }
}

#[cfg(test)]
fn move_path_with(from: &Path, to: &Path, rename: Rename) -> std::result::Result<(), MoveFailure> {
    move_path_with_report(from, to, rename).map(|_| ())
}

/// [`move_path_with`], also returning the move's warnings: a rename has none,
/// a cross-volume move may.
fn move_path_with_report(
    from: &Path,
    to: &Path,
    rename: Rename,
) -> std::result::Result<Moved, MoveFailure> {
    match rename(from, to) {
        Ok(()) => Ok(Moved::default()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(MoveFailure::Occupied),
        Err(err) if is_cross_volume(&err) => copy_verify_remove_inner(from, to),
        // A sharing violation or a permission error is real and must not be
        // papered over with a copy.
        Err(err) => Err(MoveFailure::Failed(CoreError::Quarantine(format!(
            "cannot move `{}` to `{}`: {err}",
            from.display(),
            to.display()
        )))),
    }
}

/// Whether a failed rename means "different volumes, copy it instead".
///
/// Windows reports ERROR_NOT_SAME_DEVICE (17) and everything else reports EXDEV
/// (18). The two used to be accepted interchangeably, so on Windows errno 18 —
/// ERROR_NO_MORE_FILES, nothing to do with volumes — started a copy
/// (security review L1).
fn is_cross_volume(err: &std::io::Error) -> bool {
    #[cfg(windows)]
    const CROSS_VOLUME: i32 = 17;
    #[cfg(not(windows))]
    const CROSS_VOLUME: i32 = 18;
    err.raw_os_error() == Some(CROSS_VOLUME)
}

/// Rename `from` to `to`, failing rather than replacing an occupied `to`.
///
/// `std::fs::rename` maps onto `MoveFileExW` *with* `MOVEFILE_REPLACE_EXISTING`,
/// which is exactly the wrong default for a restore: the file the user has put
/// back in the meantime is the one that would be destroyed (security review H2).
#[cfg(windows)]
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let (from_wide, to_wide) = (verbatim_wide(from)?, verbatim_wide(to)?);

    // SAFETY: both buffers are NUL-terminated and outlive the call. The flags
    // deliberately omit MOVEFILE_REPLACE_EXISTING, so an occupied destination
    // fails instead of being overwritten, and MOVEFILE_COPY_ALLOWED, so a
    // cross-volume move surfaces here and goes through `copy_verify_remove`,
    // which proves the copy byte for byte before removing anything.
    let moved = unsafe {
        MoveFileExW(
            PCWSTR(from_wide.as_ptr()),
            PCWSTR(to_wide.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    // Read the OS error directly rather than unpacking the HRESULT: it is what
    // maps ERROR_ALREADY_EXISTS onto `ErrorKind::AlreadyExists`, which the
    // caller branches on.
    moved.map_err(|_| std::io::Error::last_os_error())
}

/// `path` made absolute and spelled `\\?\C:\...` (or `\\?\UNC\server\...`),
/// NUL-terminated for a wide Win32 call.
///
/// Without the prefix `MoveFileExW` stops at MAX_PATH, so a quarantine item
/// deep enough failed every same-volume move with "path not found"
/// (cross-volume review, item 6). The prefix also turns off the rewriting
/// Win32 does to a name, so the path is made absolute (resolving `.`, `..` and
/// `/`) first, which is the same rewriting the call did before.
#[cfg(windows)]
fn verbatim_wide(path: &Path) -> std::io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    const VERBATIM: &str = r"\\?\";
    const DEVICE: &str = r"\\.\";
    const UNC: &str = r"\\";
    const VERBATIM_UNC: &str = r"\\?\UNC\";

    let absolute = std::path::absolute(path)?;
    let wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let starts_with = |prefix: &str| {
        let prefix: Vec<u16> = prefix.encode_utf16().collect();
        wide.starts_with(&prefix)
    };
    let (prefix, rest) = if starts_with(VERBATIM) || starts_with(DEVICE) {
        ("", &wide[..])
    } else if starts_with(UNC) {
        (VERBATIM_UNC, &wide[UNC.len()..])
    } else {
        (VERBATIM, &wide[..])
    };
    Ok(prefix
        .encode_utf16()
        .chain(rest.iter().copied())
        .chain(std::iter::once(0))
        .collect())
}

#[cfg(not(windows))]
#[allow(clippy::disallowed_methods)]
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    // No portable atomic no-clobber rename exists; a probe plus a rename is the
    // best this target can do, and this app ships for Windows.
    if is_occupied(to) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "the destination is occupied",
        ));
    }
    fs::rename(from, to)
}

/// Cross-volume move, reported as a [`CoreError`].
///
/// The shape the tests assert on. Production goes through
/// [`copy_verify_remove_inner`], because [`move_path_with`] has to tell
/// "occupied" apart from "failed" — see L8 on the inner function.
#[cfg(test)]
fn copy_verify_remove(from: &Path, to: &Path) -> Result<()> {
    copy_verify_remove_inner(from, to)
        .map(|_| ())
        .map_err(|failure| failure.into_error(from, to))
}

/// Cross-volume move: pin the source, copy it, prove the copy, then delete
/// exactly what was pinned.
///
/// The order matters. If the copy or its verification fails, the original is
/// still whole and only what this call created is cleaned up, so a bad move
/// costs the user nothing. Once any source file has been deleted the move has
/// happened: it reports success and logs whatever stayed behind, because an
/// error would make `take` drop the ledger row of the only complete copy
/// (security review C2).
///
/// Returns [`MoveFailure`] rather than a plain error so an occupied destination
/// stays distinguishable. It used to be flattened into `MoveFailure::Failed`,
/// which meant `move_out`'s retry loop never fired on this branch: a restore
/// onto a contested name errored out where the ` (restored)` suffix would have
/// worked (security review L8).
fn copy_verify_remove_inner(from: &Path, to: &Path) -> std::result::Result<Moved, MoveFailure> {
    let report = move_across_volumes(from, to)?;
    tracing::debug!(
        source = %from.display(),
        deleted = report.deleted_files,
        left_behind = report.leftovers.len(),
        warnings = report.warnings.len(),
        "moved across volumes"
    );
    Ok(Moved {
        warnings: report.warnings,
        left_bytes: report.left_bytes,
    })
}

/// One source file, held open from before the copy until its delete.
struct PinnedFile {
    /// Where it is in the source.
    path: PathBuf,
    /// Where it goes, relative to the destination root; empty for a lone file.
    relative: PathBuf,
    /// The pin: read and delete access, sharing only reading.
    handle: fs::File,
    /// Its identity, read through the pin.
    id: FileId,
    /// Its metadata, read through the pin before anything was copied.
    metadata: fs::Metadata,
    /// Its named data streams, listed through the pin before anything was
    /// copied. Only the mark of the web is ever among them; a file with any
    /// other is refused (cross-volume review, item 2).
    streams: Vec<pinned::DataStream>,
    /// Its mark of the web, pinned with it, if it has one.
    zone: Option<ZoneStream>,
}

/// The `Zone.Identifier` stream of a pinned file: the mark of the web that
/// makes Windows warn before running a download.
struct ZoneStream {
    /// What follows the file's own name to name the stream, spelled as the
    /// source spells it: `:Zone.Identifier`.
    suffix: String,
    /// The pin: read access, sharing only reading.
    handle: fs::File,
}

/// One source directory, held open from before the copy until its delete, so
/// it cannot be renamed, deleted or swapped for a junction meanwhile
/// (cross-volume review, item 3).
struct PinnedDir {
    /// Where it is in the source.
    path: PathBuf,
    /// Where it goes, relative to the destination root (empty for the root).
    relative: PathBuf,
    /// The pin: read and delete access, sharing reading and writing but not
    /// deleting.
    handle: fs::File,
    /// Its metadata, read through the pin before anything was copied.
    metadata: fs::Metadata,
}

/// One entry of the source, in pre-order: every directory before what is in it.
enum SourceEntry {
    /// A directory, pinned.
    Dir(PinnedDir),
    /// A file, pinned.
    File(PinnedFile),
}

/// The whole source, pinned.
struct PinnedSource {
    entries: Vec<SourceEntry>,
    is_dir: bool,
}

/// What walking the source found.
#[derive(Default)]
struct Walk {
    entries: Vec<SourceEntry>,
    /// Links, refused rather than followed (security review M7).
    links: Vec<PathBuf>,
    /// Where the root directory really is, read through its pin; every other
    /// entry must turn out to be beneath it.
    root: Option<PathBuf>,
}

/// A file this call created, with the identity of the handle that created it
/// (`None` when that handle would not say, in which case cleanup leaves it).
struct CreatedFile {
    path: PathBuf,
    id: Option<FileId>,
}

/// Everything this call created at the destination, in creation order.
#[derive(Default)]
struct Created {
    files: Vec<CreatedFile>,
    dirs: Vec<PathBuf>,
}

/// What a move that happened tells its caller: what the user should hear
/// about, and how many bytes may still be where it moved from.
#[derive(Debug, Default)]
struct Moved {
    warnings: Vec<MoveWarning>,
    left_bytes: u64,
}

/// What a finished move removed from the source, and what it left there.
#[derive(Debug, Default)]
struct MoveReport {
    deleted_files: usize,
    leftovers: Vec<PathBuf>,
    /// What the user must be told: every leftover, and anything that went
    /// other than planned (adversarial review follow-up A).
    warnings: Vec<MoveWarning>,
    /// Bytes of the files that may still be where they were moved from, so a
    /// take does not count them as reclaimed (round-2 review).
    left_bytes: u64,
}

impl MoveReport {
    fn warn(&mut self, path: &Path, reason: &str) {
        self.warnings.push(MoveWarning {
            path: path.display().to_string(),
            reason: reason.to_owned(),
        });
    }

    /// A file that may still be at the source: warned about, listed as a
    /// leftover and not counted as moved.
    fn keep(&mut self, path: PathBuf, size: u64, reason: &str) {
        self.warn(&path, reason);
        self.left_bytes += size;
        self.leftovers.push(path);
    }

    /// Whether anything has left the source: from then on the move has
    /// happened and its copy must stay (security review C2).
    fn touched_source(&self) -> bool {
        self.deleted_files > 0 || !self.warnings.is_empty()
    }
}

/// Why a cross-volume move warns about a path. Worded for either direction,
/// since a take moves out of the user's folder and a restore moves back into
/// it (adversarial review follow-up A).
const LEFT_FILE: &str = "part of the folder stayed where it was moved from: \
                         this file could not be removed there";
const LEFT_DIR: &str = "part of the folder stayed where it was moved from: \
                        this folder was not empty or had changed";
const COPY_ONLY: &str = "its removal was reported as failed, but it is gone from where \
                         it was moved from; only the moved copy remains";
const STREAM_LOST: &str = "a stream added to it during the move could not be kept; \
                           only the copy made before that remains";
const STREAM_UNCONFIRMED: &str = "a stream was added to it during the move, and taking \
     back its removal could not be confirmed; check whether it is still where it was moved \
     from. The moved copy was kept";
const BOTH_KEPT: &str = "its removal was refused and whether it is still where it was \
                         moved from could not be told; it and the moved copy were both \
                         kept, so check both places before removing either";

fn failed(message: String) -> MoveFailure {
    MoveFailure::Failed(CoreError::Quarantine(message))
}

/// `root` joined with `relative`, where an empty `relative` is `root` itself
/// (`Path::join("")` would add a trailing separator).
fn join_relative(root: &Path, relative: &Path) -> PathBuf {
    if relative.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    }
}

/// Whether `path`, a final path read from an open handle, is `root` or sits
/// beneath it, by whole components and ignoring case as NTFS does.
///
/// Lexical on purpose: both sides were read from handles held open, so they
/// already say where the files really are. Resolving them again through
/// [`path_compare::is_inside`] would re-read names that may have changed since.
fn is_within_root(root: &Path, path: &Path) -> bool {
    path_compare::CanonicalPath::lexical(path)
        .is_within(&path_compare::CanonicalPath::lexical(root))
}

/// Pin, copy, verify, delete. See [`copy_verify_remove_inner`].
fn move_across_volumes(from: &Path, to: &Path) -> std::result::Result<MoveReport, MoveFailure> {
    let source = pin_source(from)?;
    // What a failed copy may clean up is what *this call* created, and only
    // the copy itself knows that. Probing `to` first answers a different
    // question: a file that appears between the probe and `create_new` makes
    // the copy fail with `AlreadyExists` while the probe still says "did not
    // exist", and on the restore path `to` is a user path — so the cleanup
    // deleted a file this call never created (security review H1, then M10).
    let mut created = Created::default();
    if let Err(err) = copy_pinned(&source, to, &mut created) {
        cleanup_created(&created);
        return Err(copy_failure(&err, from, to));
    }
    if !copy_is_verified(&source, from, to) {
        cleanup_created(&created);
        return Err(MoveFailure::Failed(CoreError::VerifyFailed(
            from.display().to_string(),
        )));
    }
    #[cfg(test)]
    seams::run_after_verify(from);
    delete_source(from, source, &created)
}

/// Walk the source without following links and pin every file in it, before
/// anything is created at the destination. Each pin stays open until its file
/// is deleted, so from here on no other program can write, rename or delete
/// it, and one that already has it open for writing makes this fail
/// (security review C).
fn pin_source(from: &Path) -> std::result::Result<PinnedSource, MoveFailure> {
    let metadata = fs::symlink_metadata(from)
        .map_err(|err| failed(format!("cannot inspect `{}`: {err}", from.display())))?;
    if metadata.file_type().is_symlink() {
        // `take` and `move_user_file` refuse links before they get here, and a
        // restore only moves what the quarantine holds, so this is a backstop.
        // The old copy followed a link to a file and then removed the link.
        return Err(failed(format!(
            "`{}` is a link, which cannot be copied to another volume; it was left where it is",
            from.display()
        )));
    }
    #[cfg(test)]
    seams::run_before_walk(from);
    let mut walk = Walk::default();
    if metadata.is_dir() {
        walk_dir(from, Path::new(""), &mut walk)?;
    } else {
        walk.entries
            .push(SourceEntry::File(pin_file(from, PathBuf::new(), None)?));
    }
    // A link inside the tree cannot be copied, so removing the source would
    // destroy it — the copy would silently turn a link into nothing, and a
    // restore could never put it back. Refuse the whole move and leave the
    // original exactly where it is (security review M7).
    if !walk.links.is_empty() {
        return Err(failed(format!(
            "`{}` contains links that cannot be copied to another volume ({}); \
             it was left where it is",
            from.display(),
            walk.links
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(PinnedSource {
        entries: walk.entries,
        is_dir: metadata.is_dir(),
    })
}

/// Record `dir` and everything under it in pre-order, pinning files and
/// collecting links instead of following them. An entry that cannot be read
/// fails the walk rather than being skipped.
fn walk_dir(dir: &Path, relative: &Path, walk: &mut Walk) -> std::result::Result<(), MoveFailure> {
    let unreadable = |err: std::io::Error| {
        failed(format!(
            "cannot read `{}`: {err}; it was left where it is",
            dir.display()
        ))
    };
    // Pinned before it is listed, so what is listed is this very directory
    // and not a junction put in its place since it was inspected.
    let pinned_dir = pin_dir(dir, relative, walk.root.as_deref())?;
    if walk.root.is_none() {
        walk.root = pinned::final_path(&pinned_dir.handle).map_err(unreadable)?;
    }
    let mut entries = fs::read_dir(dir)
        .map_err(unreadable)?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(unreadable)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    walk.entries.push(SourceEntry::Dir(pinned_dir));
    for entry in entries {
        let path = entry.path();
        let child = relative.join(entry.file_name());
        let kind = entry.file_type().map_err(unreadable)?;
        if kind.is_symlink() {
            walk.links.push(path);
        } else if kind.is_dir() {
            walk_dir(&path, &child, walk)?;
        } else {
            let root = walk.root.as_deref();
            walk.entries
                .push(SourceEntry::File(pin_file(&path, child, root)?));
        }
    }
    Ok(())
}

/// Pin one source directory, refusing it if it is a link or reparse point,
/// carries a named stream, or (below the root) is not beneath `root`.
fn pin_dir(
    dir: &Path,
    relative: &Path,
    root: Option<&Path>,
) -> std::result::Result<PinnedDir, MoveFailure> {
    let cannot = |err: std::io::Error| pin_failure(dir, &err);
    let handle = pinned::open_pinned_dir(dir).map_err(cannot)?;
    if pinned::is_reparse_point(&handle).map_err(cannot)? {
        return Err(failed(format!(
            "`{}` is a link or reparse point, which cannot be copied to another volume; \
             it was left where it is",
            dir.display()
        )));
    }
    let metadata = handle.metadata().map_err(cannot)?;
    if !metadata.is_dir() {
        return Err(failed(format!(
            "`{}` is no longer a directory; it was left where it is",
            dir.display()
        )));
    }
    if pinned::is_encrypted(&metadata) {
        return Err(encrypted(dir));
    }
    if !pinned::named_streams(&handle).map_err(cannot)?.is_empty() {
        return Err(failed(format!(
            "`{}` carries an alternate data stream, which cannot be copied to another \
             volume; it was left where it is",
            dir.display()
        )));
    }
    check_contained(dir, &handle, root)?;
    Ok(PinnedDir {
        path: dir.to_path_buf(),
        relative: relative.to_path_buf(),
        handle,
        metadata,
    })
}

/// Refuse an entry whose pin says it is not beneath the root being moved:
/// a directory on the way to it was swapped for a link after it was listed.
/// Nothing to check for a lone file, or where a handle cannot say where it is.
fn check_contained(
    path: &Path,
    handle: &fs::File,
    root: Option<&Path>,
) -> std::result::Result<(), MoveFailure> {
    let Some(root) = root else {
        return Ok(());
    };
    let Some(actual) = pinned::final_path(handle).map_err(|err| pin_failure(path, &err))? else {
        return Ok(());
    };
    if is_within_root(root, &actual) {
        return Ok(());
    }
    Err(failed(format!(
        "`{}` leads outside the folder being moved (to `{}`), through a link put \
         in place during the move; it was left where it is",
        path.display(),
        actual.display()
    )))
}

/// The mark of the web, as a stream listing names it.
const ZONE_STREAM: &str = ":zone.identifier:$data";
/// What a listed stream name ends with: its type.
const DATA_STREAM_TYPE: &str = ":$DATA";

/// Pin one source file, refusing it if it turns out to be a reparse point or
/// carries a named stream other than the mark of the web. Below a directory
/// root, it must also turn out to be beneath that root.
fn pin_file(
    path: &Path,
    relative: PathBuf,
    root: Option<&Path>,
) -> std::result::Result<PinnedFile, MoveFailure> {
    let cannot = |err: std::io::Error| pin_failure(path, &err);
    let handle = pinned::open_pinned(path).map_err(cannot)?;
    if pinned::is_reparse_point(&handle).map_err(cannot)? {
        return Err(failed(format!(
            "`{}` is a reparse point (a cloud, deduplicated or compressed placeholder) \
             and cannot be copied faithfully to another volume; it was left where it is",
            path.display()
        )));
    }
    let metadata = handle.metadata().map_err(cannot)?;
    if pinned::is_encrypted(&metadata) {
        return Err(encrypted(path));
    }
    if pinned::standard_info(&handle).map_err(cannot)?.links > 1 {
        // Its other name would keep the data in place while this one moved,
        // and the copy would be a second, separate file (adversarial review,
        // LOW 3). Refused for what it is rather than as a sharing failure.
        return Err(failed(format!(
            "`{}` has more than one name (hard link); WinKnife does not move \
             hard-linked files across drives; it was left where it is",
            path.display()
        )));
    }
    check_contained(path, &handle, root)?;
    let id = pinned::handle_identity(&handle).map_err(cannot)?;
    let streams = pinned::named_streams(&handle).map_err(cannot)?;
    let mut zone = None;
    for stream in &streams {
        if !stream.name.eq_ignore_ascii_case(ZONE_STREAM) {
            // A copy of the main stream drops it, and deleting the original
            // then destroys it (cross-volume review, item 2).
            return Err(failed(format!(
                "`{}` carries an alternate data stream (`{}`), which cannot be copied \
                 to another volume; it was left where it is",
                path.display(),
                stream.name
            )));
        }
        let suffix = stream.name[..stream.name.len() - DATA_STREAM_TYPE.len()].to_string();
        let stream_handle =
            pinned::open_pinned_stream(&with_suffix(path, &suffix)).map_err(cannot)?;
        zone = Some(ZoneStream {
            suffix,
            handle: stream_handle,
        });
    }
    Ok(PinnedFile {
        path: path.to_path_buf(),
        relative,
        handle,
        id,
        metadata,
        streams,
        zone,
    })
}

/// The refusal for an EFS-encrypted file or directory: a copy on another
/// drive is written in the clear (adversarial review, LOW 2). A rename on one
/// volume never gets here and keeps the encryption.
fn encrypted(path: &Path) -> MoveFailure {
    MoveFailure::Encrypted(CoreError::Quarantine(format!(
        "`{}` {EFS_REFUSAL}; moving it across drives would store \
         it unencrypted; it was left where it is",
        path.display()
    )))
}

/// What the EFS refusal says. A restore recognises the refusal by its
/// [`MoveFailure::Encrypted`] variant, not by this wording (round-2 review).
const EFS_REFUSAL: &str = "is encrypted with Windows EFS";

/// `path` with `suffix` added to its last component: `a.txt:Zone.Identifier`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut joined = path.as_os_str().to_os_string();
    joined.push(suffix);
    PathBuf::from(joined)
}

#[cfg(windows)]
fn is_sharing_violation(err: &std::io::Error) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    err.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

#[cfg(not(windows))]
fn is_sharing_violation(_err: &std::io::Error) -> bool {
    false
}

/// A user-worded refusal for a file that could not be pinned. Nothing exists
/// at the destination yet, so there is nothing to undo.
fn pin_failure(path: &Path, err: &std::io::Error) -> MoveFailure {
    if is_sharing_violation(err) {
        return failed(format!(
            "`{}` is open in another program; it was left where it is",
            path.display()
        ));
    }
    failed(format!(
        "cannot open `{}` to move it: {err}; it was left where it is",
        path.display()
    ))
}

/// Copy the pinned source to `to`, parents first, recording everything this
/// call creates the moment it exists.
fn copy_pinned(source: &PinnedSource, to: &Path, created: &mut Created) -> std::io::Result<()> {
    for entry in &source.entries {
        match entry {
            SourceEntry::Dir(dir) => {
                let target = join_relative(to, &dir.relative);
                // `create_dir` fails rather than succeeding on an existing
                // directory, so reaching the next line means this call made it.
                fs::create_dir(&target)?;
                #[cfg(test)]
                seams::run_after_create_dir(&target);
                created.dirs.push(target);
            }
            SourceEntry::File(file) => {
                copy_pinned_file(file, &join_relative(to, &file.relative), created)?;
            }
        }
    }
    // Directory attributes last and deepest first, so a read-only or hidden
    // directory is complete before it is marked (cross-volume review, item 4).
    for entry in source.entries.iter().rev() {
        if let SourceEntry::Dir(dir) = entry {
            copy_kept_dir_attributes(&join_relative(to, &dir.relative), &dir.metadata)?;
        }
    }
    Ok(())
}

/// Copy one pinned file to a destination that must not already exist, reading
/// through the pin and keeping the original's timestamps and read-only flag.
///
/// `fs::copy` would happily truncate whatever is at `to`, and it is the
/// exclusive claim on `to` that makes a restore safe (security review H2).
/// A cross-volume copy that lost the timestamps would also make a restored
/// file look brand new to every backup and every age-based rule (M6). The new
/// file is recorded the instant it exists, with the identity of the handle
/// that created it, because that record is all a failed move may clean up
/// (M10, then D).
fn copy_pinned_file(file: &PinnedFile, to: &Path, created: &mut Created) -> std::io::Result<()> {
    // Read access too: the identity, metadata and attributes below are all
    // asked for through this handle.
    let mut destination = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(to)?;
    // `create_new` succeeded, so nothing was at `to` and this call is what
    // put something there. Anything that fails from here on may be cleaned
    // up; anything that failed before this line may not.
    let id = pinned::handle_identity(&destination);
    created.files.push(CreatedFile {
        path: to.to_path_buf(),
        id: id.as_ref().ok().copied(),
    });
    // Without its identity a cleanup could not tell this file from one
    // swapped in under its name, so it would have to leave it: stop here.
    id?;
    let mut reader = &file.handle;
    std::io::copy(&mut reader, &mut destination)?;
    destination.sync_all()?;
    if let Some(zone) = &file.zone {
        copy_zone(zone, to)?;
    }
    // After every stream is written, since writing one moves the file's time.
    let modified = file.metadata.modified()?;
    destination.set_times(
        fs::FileTimes::new()
            .set_modified(modified)
            .set_accessed(file.metadata.accessed().unwrap_or(modified)),
    )?;
    // Last, so nothing above has to write to a read-only file.
    copy_kept_attributes(&destination, &file.metadata)
}

/// Set exactly the kept attributes of `source` on the open `file`, clearing
/// any it should not have (a fresh copy is marked for archiving, for one).
/// Last in a copy, because a read-only file cannot be written to after.
#[cfg(windows)]
fn copy_kept_attributes(file: &fs::File, source: &std::fs::Metadata) -> std::io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileBasicInfo, SetFileInformationByHandle, FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO,
    };

    // What the copy has that a move does not carry stays as it is.
    let others = file.metadata()?.file_attributes() & !pinned::KEPT_ATTRIBUTES;
    let wanted = others | pinned::kept_attributes(source);
    let info = FILE_BASIC_INFO {
        // Zero times mean "leave unchanged"; zero attributes would too, so
        // "none at all" is spelled NORMAL.
        FileAttributes: if wanted == 0 {
            FILE_ATTRIBUTE_NORMAL.0
        } else {
            wanted
        },
        ..FILE_BASIC_INFO::default()
    };
    // SAFETY: the handle is owned by `file` and outlives the call, and the
    // buffer is a live FILE_BASIC_INFO whose exact size is passed.
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileBasicInfo,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    }
    .map_err(|_| std::io::Error::last_os_error())
}

/// Elsewhere the read-only flag is all there is to carry.
#[cfg(not(windows))]
fn copy_kept_attributes(file: &fs::File, source: &std::fs::Metadata) -> std::io::Result<()> {
    if !source.permissions().readonly() {
        return Ok(());
    }
    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions)
}

/// [`copy_kept_attributes`] for a directory, opened by `path` as itself.
#[cfg(windows)]
fn copy_kept_dir_attributes(path: &Path, source: &fs::Metadata) -> std::io::Result<()> {
    copy_kept_attributes(&pinned::open_dir_attributes(path)?, source)
}

/// Elsewhere a directory keeps no attributes across a move, as before.
#[cfg(not(windows))]
fn copy_kept_dir_attributes(_path: &Path, _source: &std::fs::Metadata) -> std::io::Result<()> {
    Ok(())
}

/// Copy the mark of the web next to a freshly created copy, reading through
/// its pin. The copy is this call's own, so the stream cannot already exist.
fn copy_zone(zone: &ZoneStream, to: &Path) -> std::io::Result<()> {
    let mut stream = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(with_suffix(to, &zone.suffix))?;
    let mut reader = &zone.handle;
    std::io::Seek::seek(&mut reader, std::io::SeekFrom::Start(0))?;
    std::io::copy(&mut reader, &mut stream)?;
    stream.sync_all()
}

fn copy_failure(err: &std::io::Error, from: &Path, to: &Path) -> MoveFailure {
    // `create_new` refusing an occupied destination is not a failure — it is
    // exactly the signal `move_out`'s retry loop exists for, and reporting it
    // as one is what made a contested restore give up instead of trying
    // ` (restored)` (security review L8).
    if err.kind() == std::io::ErrorKind::AlreadyExists {
        return MoveFailure::Occupied;
    }
    failed(format!(
        "cannot copy `{}` to `{}`: {err}",
        from.display(),
        to.display()
    ))
}

/// Whether the copy is proven: bytes, times and read-only flag match (failing
/// closed on anything that cannot be compared), and every source name still
/// leads to the very file that was pinned and copied.
fn copy_is_verified(source: &PinnedSource, from: &Path, to: &Path) -> bool {
    let compared = if source.is_dir {
        dirs_are_identical(from, to)
    } else {
        copies_match(from, to)
    };
    compared.unwrap_or(false)
        && source.entries.iter().all(|entry| match entry {
            SourceEntry::File(file) => {
                platform::file_identity(&file.path) == Some(file.id)
                    && streams_are_copied(file, &join_relative(to, &file.relative))
            }
            SourceEntry::Dir(_) => true,
        })
}

/// Whether the copy carries exactly the named streams the pinned file had,
/// the mark of the web byte for byte (cross-volume review, item 2).
fn streams_are_copied(file: &PinnedFile, copy: &Path) -> bool {
    let listed = fs::File::open(copy).and_then(|copy| pinned::named_streams(&copy));
    if !listed.is_ok_and(|listed| listed == file.streams) {
        return false;
    }
    let Some(zone) = &file.zone else {
        return true;
    };
    let mut original = Vec::new();
    let mut reader = &zone.handle;
    let read = std::io::Seek::seek(&mut reader, std::io::SeekFrom::Start(0))
        .and_then(|_| reader.read_to_end(&mut original));
    read.is_ok() && fs::read(with_suffix(copy, &zone.suffix)).is_ok_and(|copied| copied == original)
}

/// The source's pinned directories (parents first) and its pinned files.
fn split_entries(entries: Vec<SourceEntry>) -> (Vec<PinnedDir>, Vec<PinnedFile>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries {
        match entry {
            SourceEntry::Dir(dir) => dirs.push(dir),
            SourceEntry::File(file) => files.push(file),
        }
    }
    (dirs, files)
}

/// Delete exactly the files that were pinned, copied and verified, through
/// the handles held since before the copy, then each source directory that is
/// empty by then (invariants I1 and I2).
fn delete_source(
    from: &Path,
    source: PinnedSource,
    created: &Created,
) -> std::result::Result<MoveReport, MoveFailure> {
    let (dirs, files) = split_entries(source.entries);
    let mut report = MoveReport::default();
    for (index, file) in files.into_iter().enumerate() {
        let outcome = match injected_delete_failure(index, &file) {
            Some(err) => Err(err),
            None => delete_pinned(&file),
        };
        // `file` is dropped at the end of this iteration, which closes its
        // pin: that is the moment a delete takes effect.
        match outcome {
            Ok(()) => report.deleted_files += 1,
            // Gone with a stream that arrived during the move: not a delete
            // that went as asked, but the source has been touched, so the copy
            // is all there is now (adversarial review follow-up D).
            Err(err) if is_source_lost(&err) => report.warn(&file.path, STREAM_LOST),
            Err(err) if is_source_unconfirmed(&err) => {
                let size = file.metadata.len();
                report.keep(file.path, size, STREAM_UNCONFIRMED);
            }
            Err(err) if !report.touched_source() => {
                first_delete_refused(from, file, &err, created, &mut report)?;
            }
            Err(err) => {
                tracing::warn!(
                    path = %file.path.display(),
                    error = %err,
                    "could not remove a moved file; it stays at the source"
                );
                let size = file.metadata.len();
                report.keep(file.path, size, LEFT_FILE);
            }
        }
    }
    remove_pinned_dirs(dirs, &mut report);
    if !report.warnings.is_empty() {
        tracing::warn!(
            count = report.warnings.len(),
            left_behind = report.leftovers.len(),
            source = %from.display(),
            "moved, but not everything went as planned"
        );
    }
    Ok(report)
}

/// What is at a source file's name once its refused delete's pin is closed.
enum AfterRefusal {
    /// Nothing, or another file: the source went although its delete said
    /// it failed.
    Gone,
    /// The very file that was pinned.
    Present,
    /// The filesystem would not say.
    Unknown(std::io::Error),
}

/// Look at `path` after its delete was refused. Only "not found" or another
/// file there counts as gone; an identity that cannot be read is unknown, not
/// gone (adversarial review follow-up B).
fn source_after_refusal(path: &Path, id: FileId) -> AfterRefusal {
    match source_identity(path) {
        Ok(now) if now == id => AfterRefusal::Present,
        Ok(_) => AfterRefusal::Gone,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => AfterRefusal::Gone,
        Err(err) => AfterRefusal::Unknown(err),
    }
}

/// The identity of the source file at `path`, read after its delete was
/// refused.
fn source_identity(path: &Path) -> std::io::Result<FileId> {
    match injected_identity_failure() {
        Some(err) => Err(err),
        None => platform::try_file_identity(path),
    }
}

/// The first delete was refused and nothing has left the source yet. Close
/// the pin before touching the copy, then look: a source that went when it
/// closed leaves only the copy, which must stay (adversarial review, LOW 1);
/// one still there makes the move undo its copy; one that cannot be told
/// keeps both (follow-up B).
fn first_delete_refused(
    from: &Path,
    file: PinnedFile,
    err: &std::io::Error,
    created: &Created,
    report: &mut MoveReport,
) -> std::result::Result<(), MoveFailure> {
    let (path, id, size) = (file.path.clone(), file.id, file.metadata.len());
    drop(file);
    match source_after_refusal(&path, id) {
        AfterRefusal::Gone => {
            tracing::error!(
                path = %path.display(),
                error = %err,
                "the source went although its delete was refused; keeping the copy"
            );
            report.deleted_files += 1;
            report.warn(&path, COPY_ONLY);
            Ok(())
        }
        AfterRefusal::Present => {
            cleanup_created(created);
            Err(failed(format!(
                "copied `{}` but could not remove the original: {err}; \
                 it was left where it is",
                from.display()
            )))
        }
        AfterRefusal::Unknown(read) => {
            // The copy may be the only one left, so the move stands and its
            // record with it; the user is told to look in both places
            // (round-2 review, HIGH).
            tracing::error!(
                path = %path.display(),
                error = %err,
                read_error = %read,
                "could not tell whether a source survived its refused delete; keeping both"
            );
            report.keep(path, size, BOTH_KEPT);
            Ok(())
        }
    }
}

/// Remove the pinned source directories deepest first, each only if it is
/// empty by then; report the ones that stay.
fn remove_pinned_dirs(dirs: Vec<PinnedDir>, report: &mut MoveReport) {
    for dir in dirs.into_iter().rev() {
        match delete_pinned_dir(&dir) {
            Ok(()) => {}
            Err(err) if is_source_lost(&err) => report.warn(&dir.path, STREAM_LOST),
            Err(err) if is_source_unconfirmed(&err) => {
                report.warn(&dir.path, STREAM_UNCONFIRMED);
                report.leftovers.push(dir.path.clone());
            }
            Err(err) => {
                tracing::warn!(
                    path = %dir.path.display(),
                    error = %err,
                    "leaving a source directory in place: it is not empty or it changed"
                );
                report.warn(&dir.path, LEFT_DIR);
                report.leftovers.push(dir.path.clone());
            }
        }
        // `dir` is dropped here, closing its pin: that is the moment its
        // delete takes effect, before its parent's is asked for.
    }
}

/// The error for a source whose delete could not be taken back after a stream
/// arrived during the move: it is gone, and that stream with it
/// (adversarial review follow-up D).
#[derive(Debug)]
struct SourceLost;

impl std::fmt::Display for SourceLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(STREAM_LOST)
    }
}

impl std::error::Error for SourceLost {}

/// Whether `err` is a [`SourceLost`].
fn is_source_lost(err: &std::io::Error) -> bool {
    err.get_ref().is_some_and(|inner| inner.is::<SourceLost>())
}

/// The error for a source whose delete was taken back after a stream arrived
/// during the move, where the read that should confirm it failed: it may
/// still be there, or may be gone (round-2 review).
#[derive(Debug)]
struct SourceUnconfirmed(std::io::Error);

impl std::fmt::Display for SourceUnconfirmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{STREAM_UNCONFIRMED} ({})", self.0)
    }
}

impl std::error::Error for SourceUnconfirmed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Whether `err` is a [`SourceUnconfirmed`].
fn is_source_unconfirmed(err: &std::io::Error) -> bool {
    err.get_ref()
        .is_some_and(|inner| inner.is::<SourceUnconfirmed>())
}

/// Delete a pinned directory through its pin. A directory that is not empty
/// refuses, which is the point: anything that arrived in it stays, and so
/// does the directory.
#[cfg(windows)]
fn delete_pinned_dir(dir: &PinnedDir) -> std::io::Result<()> {
    delete_by_handle(&dir.handle)?;
    keep_if_streams_changed(&dir.handle, &dir.path, &[])
}

/// Elsewhere there is no delete through a handle.
#[cfg(not(windows))]
fn delete_pinned_dir(dir: &PinnedDir) -> std::io::Result<()> {
    remove_empty_dir(&dir.path)
}

/// Remove `dir` only if it is still a plain, empty directory, checking and
/// removing through one handle, read-only or not.
#[cfg(windows)]
fn remove_empty_dir(dir: &Path) -> std::io::Result<()> {
    let handle = pinned::open_dir_to_delete(dir)?;
    if pinned::is_reparse_point(&handle)? || !handle.metadata()?.is_dir() {
        return Err(std::io::Error::other("no longer a plain directory"));
    }
    delete_by_handle(&handle)
}

/// Remove `dir` only if it is still a plain, empty directory.
#[cfg(not(windows))]
#[allow(clippy::disallowed_methods)]
fn remove_empty_dir(dir: &Path) -> std::io::Result<()> {
    if !fs::symlink_metadata(dir)?.is_dir() {
        return Err(std::io::Error::other("no longer a plain directory"));
    }
    // `remove_dir` refuses a directory that is not empty, which is the point:
    // anything that arrived in it stays, and so does the directory.
    fs::remove_dir(dir)
}

/// Mark an open file for deletion through its handle, so the file deleted is
/// the one the handle holds, whatever its name leads to now. POSIX semantics
/// take the name away as soon as the handle closes, and the read-only flag is
/// ignored, as a same-volume rename would. A filesystem without the extended
/// call (FAT, exFAT) gets the classic one, which refuses a read-only file:
/// that file then stays behind as a leftover.
#[cfg(windows)]
fn delete_by_handle(file: &fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileDispositionInfo, FileDispositionInfoEx, SetFileInformationByHandle,
        FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX,
        FILE_DISPOSITION_INFO_EX_FLAGS,
    };
    // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED and ERROR_INVALID_PARAMETER:
    // what a filesystem without the extended call answers.
    const NO_EXTENDED_DISPOSITION: [i32; 3] = [1, 50, 87];

    let handle = HANDLE(file.as_raw_handle());
    let extended = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
            FILE_DISPOSITION_FLAG_DELETE.0
                | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
        ),
    };
    // SAFETY: the handle is owned by `file` and outlives the call, and the
    // buffer is a live FILE_DISPOSITION_INFO_EX whose exact size is passed.
    let marked = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfoEx,
            std::ptr::from_ref(&extended).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    }
    .map_err(|_| std::io::Error::last_os_error());
    let Err(err) = marked else {
        return Ok(());
    };
    if !err
        .raw_os_error()
        .is_some_and(|code| NO_EXTENDED_DISPOSITION.contains(&code))
    {
        return Err(err);
    }
    let classic = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: as above, for a live FILE_DISPOSITION_INFO of the size passed.
    unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfo,
            std::ptr::from_ref(&classic).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|_| std::io::Error::last_os_error())
}

/// Take back a delete marked through `file`, before its handle closes, and
/// confirm through the same handle that it is no longer pending.
///
/// Why (adversarial review, LOW 1): the call can report success and leave the
/// delete in place. Believed, that made the move undo its copy as if the
/// source were whole, and the source then went when its pin closed. An error
/// here means the file goes, so the caller keeps the copy.
#[cfg(windows)]
fn undelete_by_handle(file: &fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };

    if !revoke_skipped() {
        let keep = FILE_DISPOSITION_INFO { DeleteFile: false };
        // SAFETY: the handle is owned by `file` and outlives the call, and the
        // buffer is a live FILE_DISPOSITION_INFO whose exact size is passed.
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                std::ptr::from_ref(&keep).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        }
        .map_err(|_| std::io::Error::last_os_error())?;
    }
    // The take-back went through; a read that cannot confirm it leaves the
    // source's fate unknown rather than lost (round-2 review).
    let confirmed = match injected_confirmation_failure() {
        Some(err) => Err(err),
        None => pinned::standard_info(file),
    }
    .map_err(|err| std::io::Error::other(SourceUnconfirmed(err)))?;
    if confirmed.delete_pending {
        return Err(std::io::Error::other(
            "the delete was still pending after it was taken back",
        ));
    }
    Ok(())
}

/// Once a delete is marked, check that the named streams are still exactly
/// the ones copied, and take the delete back if not.
///
/// NTFS shares by stream, so a pin on the main stream does not stop another
/// program adding or growing a named one. Checked after the mark, because from
/// then on the file cannot be opened again, so no stream can be added between
/// this check and the delete (cross-volume review, item 2).
#[cfg(windows)]
fn keep_if_streams_changed(
    handle: &fs::File,
    path: &Path,
    copied: &[pinned::DataStream],
) -> std::io::Result<()> {
    if pinned::named_streams(handle).is_ok_and(|now| now == copied) {
        return Ok(());
    }
    if let Err(err) = undelete_by_handle(handle) {
        if is_source_unconfirmed(&err) {
            // Taken back, but not confirmed: the source may still be there,
            // or may go with the stream when its pin closes. The copy stays
            // and the user is told to look (round-2 review).
            tracing::error!(
                path = %path.display(),
                error = %err,
                "could not confirm that a source whose streams changed was kept; \
                 keeping the copy"
            );
            return Err(err);
        }
        // The take-back failed or left the delete pending, so the source goes
        // when its pin closes whatever is said here, with the stream that
        // arrived. The verified copy must stay rather than be undone as if the
        // original were still whole, but this is not a delete that went as
        // asked: it is reported as a loss (adversarial review follow-up D).
        tracing::error!(
            path = %path.display(),
            error = %err,
            "a source whose streams changed after it was copied could not be kept; \
             only the copy remains"
        );
        return Err(std::io::Error::other(SourceLost));
    }
    Err(std::io::Error::other(
        "its data streams changed since it was copied",
    ))
}

#[cfg(windows)]
fn delete_pinned(file: &PinnedFile) -> std::io::Result<()> {
    delete_by_handle(&file.handle)?;
    keep_if_streams_changed(&file.handle, &file.path, &file.streams)
}

/// Elsewhere there is no delete through a handle: re-check the identity
/// right before removing by name.
#[cfg(not(windows))]
fn delete_pinned(file: &PinnedFile) -> std::io::Result<()> {
    if delete_if_same(&file.path, file.id)? {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "the file was replaced since it was copied",
        ))
    }
}

/// Delete the file at `path` only if it is still the file `id` names,
/// checking and deleting through one handle so the two cannot be about
/// different files.
#[cfg(windows)]
fn delete_if_same(path: &Path, id: FileId) -> std::io::Result<bool> {
    let handle = pinned::open_to_delete(path)?;
    if pinned::handle_identity(&handle)? != id {
        return Ok(false);
    }
    delete_by_handle(&handle)?;
    Ok(true)
}

/// Elsewhere there is no delete through a handle: re-check the identity right
/// before removing by name. The app ships for Windows; this keeps the tests
/// meaningful on other hosts.
#[cfg(not(windows))]
#[allow(clippy::disallowed_methods)]
fn delete_if_same(path: &Path, id: FileId) -> std::io::Result<bool> {
    if platform::file_identity(path) != Some(id) {
        return Ok(false);
    }
    fs::remove_file(path)?;
    Ok(true)
}

/// Undo a failed move's copy: remove what this call created and nothing else
/// (security review D).
///
/// It used to be a `remove_dir_all` of the destination, and on the restore
/// path that is in the user's folder: whatever another program put into a
/// directory this call had created went with it. Now a file goes only if it is
/// still the very file this call created, checked and removed through one
/// handle, and a directory only if it is empty, deepest first. Anything else
/// stays where it is and is logged.
fn cleanup_created(created: &Created) {
    for file in created.files.iter().rev() {
        let removed = match file.id {
            Some(id) => delete_if_same(&file.path, id),
            None => Ok(false),
        };
        match removed {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                path = %file.path.display(),
                "leaving a file in place after a failed copy: it cannot be shown to be the one this call created"
            ),
            Err(err) => tracing::error!(
                path = %file.path.display(),
                error = %err,
                "could not clean up a half-written copy; it is left behind rather than retried blindly"
            ),
        }
    }
    for dir in created.dirs.iter().rev() {
        if let Err(err) = remove_empty_dir(dir) {
            tracing::warn!(
                path = %dir.display(),
                error = %err,
                "leaving a directory in place after a failed copy: it is not empty or no longer ours"
            );
        }
    }
}

/// The single-file copy in the shape the older tests use: report whether
/// this call created `to`, which is what decides whether a failed copy may be
/// cleaned up (security review M10).
#[cfg(test)]
fn copy_file(from: &Path, to: &Path, created_destination: &mut bool) -> std::io::Result<()> {
    let file = pin_file(from, PathBuf::new(), None)
        .map_err(|failure| std::io::Error::other(failure.into_error(from, to).to_string()))?;
    let mut created = Created::default();
    let copied = copy_pinned_file(&file, to, &mut created);
    *created_destination = !created.files.is_empty();
    copied
}

/// The tree copy in the shape the older tests use, returning the links it
/// refused to follow (security review M7).
#[cfg(test)]
fn copy_dir_recursive(
    from: &Path,
    to: &Path,
    created_destination: &mut bool,
) -> std::io::Result<Vec<PathBuf>> {
    let mut walk = Walk::default();
    walk_dir(from, Path::new(""), &mut walk)
        .map_err(|failure| std::io::Error::other(failure.into_error(from, to).to_string()))?;
    let source = PinnedSource {
        entries: walk.entries,
        is_dir: true,
    };
    let mut created = Created::default();
    let copied = copy_pinned(&source, to, &mut created);
    *created_destination = !created.dirs.is_empty();
    copied.map(|()| walk.links)
}

/// The cleanup in the shape the older tests use, over [`cleanup_created`]:
/// remove a destination this call created, and nothing else.
#[cfg(test)]
fn cleanup_partial(path: &Path, created_by_this_call: bool) {
    if !created_by_this_call {
        tracing::warn!(
            path = %path.display(),
            "leaving the destination alone after a failed copy: this call did not create it"
        );
        return;
    }
    let created = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Created {
            files: Vec::new(),
            dirs: vec![path.to_path_buf()],
        },
        Ok(_) => Created {
            files: vec![CreatedFile {
                path: path.to_path_buf(),
                id: platform::file_identity(path),
            }],
            dirs: Vec::new(),
        },
        Err(_) => return,
    };
    cleanup_created(&created);
}

/// Move one user file to a new place, without ever overwriting or deleting.
///
/// # Why this is here and not in the module that wants it
///
/// The startup manager disables a Startup-folder shortcut by moving the `.lnk`
/// into a module-managed folder (module spec 05), and that is a move of user
/// data. `clippy.toml` forbids `fs::rename` and `fs::remove_*` everywhere but
/// this file, precisely so there is one audited implementation of "move
/// something the user owns": rename on the same volume, copy-verify-remove
/// across volumes, and never replace an occupied destination.
///
/// This is not a quarantine take — there is no batch, no ledger row and no
/// retention, because a disabled shortcut must never be purged. The startup
/// module keeps its own ledger so the move is reversible; what it borrows here
/// is the primitive, not the policy.
///
/// # Why a module cannot call this
///
/// It used to be `pub`, and the startup manager called it and then wrote its
/// ledger row afterwards — the one order that turns a failed write into a file
/// nobody can find (security review H11). It is visible only inside `core` now,
/// and the way in from a module is [`crate::core::ledger::move_user_file`],
/// which demands a [`crate::core::ledger::Recorded`] that only a committed
/// ledger row can produce.
///
/// # Errors
/// Returns [`CoreError::Quarantine`] when the source is missing, the
/// destination is occupied, or the move fails; [`CoreError::VerifyFailed`] when
/// a cross-volume copy did not match, in which case the original is untouched.
/// On success, what the user should know about a move that happened only in
/// part.
pub(in crate::core) fn move_user_file(from: &Path, to: &Path) -> Result<Vec<MoveWarning>> {
    let metadata = fs::symlink_metadata(from).map_err(|err| {
        CoreError::Quarantine(format!("cannot inspect `{}`: {err}", from.display()))
    })?;
    if metadata.is_symlink() {
        // Same refusal as `take`: moving a link moves the link, not the data.
        return Err(CoreError::Quarantine(format!(
            "`{}` is a symbolic link; links are never moved",
            from.display()
        )));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            CoreError::Quarantine(format!("cannot create `{}`: {err}", parent.display()))
        })?;
    }
    // A move that happened only in part says so; the caller hands it on
    // rather than dropping it (round-2 review).
    move_path_reporting(from, to)
        .map(|moved| moved.warnings)
        .map_err(|failure| failure.into_error(from, to))
}

/// Byte-for-byte comparison. Deliberately not a hash: no dependency, no
/// collision argument to make, and quarantine moves are not hot-path work.
pub fn files_are_identical(a: &Path, b: &Path) -> std::io::Result<bool> {
    let (meta_a, meta_b) = (fs::metadata(a)?, fs::metadata(b)?);
    if meta_a.len() != meta_b.len() {
        return Ok(false);
    }

    let mut reader_a = std::io::BufReader::new(fs::File::open(a)?);
    let mut reader_b = std::io::BufReader::new(fs::File::open(b)?);
    let mut buf_a = vec![0u8; COPY_BUFFER_BYTES];
    let mut buf_b = vec![0u8; COPY_BUFFER_BYTES];

    loop {
        let read_a = read_full(&mut reader_a, &mut buf_a)?;
        let read_b = read_full(&mut reader_b, &mut buf_b)?;
        if read_a != read_b || buf_a[..read_a] != buf_b[..read_b] {
            return Ok(false);
        }
        if read_a == 0 {
            return Ok(true);
        }
    }
}

fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// Whether `b` is a faithful copy of `a`: same bytes, same modification time,
/// same kept attributes (read-only, hidden, system, archive, not indexed).
///
/// Bytes alone used to be the whole test, which passed a copy that had silently
/// dropped every timestamp and attribute the original carried
/// (security review M6).
fn copies_match(a: &Path, b: &Path) -> std::io::Result<bool> {
    if !files_are_identical(a, b)? {
        return Ok(false);
    }

    let (meta_a, meta_b) = (fs::metadata(a)?, fs::metadata(b)?);
    if pinned::kept_attributes(&meta_a) != pinned::kept_attributes(&meta_b) {
        return Ok(false);
    }

    match (meta_a.modified(), meta_b.modified()) {
        (Ok(one), Ok(other)) => {
            let apart = if one > other {
                one.duration_since(other)
            } else {
                other.duration_since(one)
            };
            Ok(apart.is_ok_and(|apart| apart <= TIME_TOLERANCE))
        }
        // A filesystem that will not report modification times cannot disagree
        // about them either.
        _ => Ok(true),
    }
}

/// The names in a directory listing, sorted so two listings compare as sets.
///
/// An entry that cannot be read fails the whole listing. Skipping it used to
/// let verification pass without ever comparing that entry, and the source was
/// then removed on the strength of a check that never looked at it.
fn sorted_names(
    entries: impl Iterator<Item = std::io::Result<std::ffi::OsString>>,
) -> std::io::Result<Vec<std::ffi::OsString>> {
    let mut names = entries.collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

fn dirs_are_identical(a: &Path, b: &Path) -> std::io::Result<bool> {
    let names = |dir: &Path| -> std::io::Result<Vec<std::ffi::OsString>> {
        sorted_names(fs::read_dir(dir)?.map(|entry| entry.map(|entry| entry.file_name())))
    };
    let kept = |dir: &Path| fs::symlink_metadata(dir).map(|meta| pinned::kept_attributes(&meta));
    if kept(a)? != kept(b)? {
        return Ok(false);
    }

    let entries = names(a)?;
    if entries != names(b)? {
        return Ok(false);
    }

    for name in entries {
        let (child_a, child_b) = (a.join(&name), b.join(&name));
        // `symlink_metadata`, not `metadata`: a link and the file it points at
        // are different things, and comparing through the link would call a
        // dereferenced copy identical to the link it replaced
        // (security review M7).
        let (kind_a, kind_b) = (
            fs::symlink_metadata(&child_a)?.file_type(),
            fs::symlink_metadata(&child_b)?.file_type(),
        );
        if kind_a.is_symlink() != kind_b.is_symlink() || kind_a.is_dir() != kind_b.is_dir() {
            return Ok(false);
        }

        let same = if kind_a.is_dir() {
            dirs_are_identical(&child_a, &child_b)?
        } else {
            copies_match(&child_a, &child_b)?
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Recursive size of a directory.
///
/// A directory we cannot read used to contribute zero, which made an unreadable
/// tree look weightless: the ledger recorded nothing, and the size cap then
/// evicted real items to reclaim space that was never counted
/// (security review M4). It is an error now.
fn directory_size(path: &Path) -> Result<u64> {
    let unreadable = |err: std::io::Error| {
        CoreError::Quarantine(format!("cannot measure `{}`: {err}", path.display()))
    };

    let mut total = 0u64;
    for entry in fs::read_dir(path).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let kind = entry.file_type().map_err(unreadable)?;
        total = total.saturating_add(if kind.is_symlink() {
            // A link holds no bytes of its own.
            0
        } else if kind.is_dir() {
            directory_size(&entry.path())?
        } else {
            entry.metadata().map_err(unreadable)?.len()
        });
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    struct Harness {
        _dir: tempfile::TempDir,
        conn: Connection,
        quarantine: Quarantine,
        work: PathBuf,
    }

    fn harness(policy: RetentionPolicy) -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        Harness {
            conn: db::open_in_memory().expect("db"),
            quarantine: Quarantine::with_root(dir.path().join("quarantine"), policy),
            work,
            _dir: dir,
        }
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(&path, bytes).expect("write");
        path
    }

    /// Security review M13: `destroy` is the one function in this crate allowed
    /// to remove user data for good, and its root guard was
    /// `Path::starts_with`, which counts `..` as an ordinary component — so
    /// `…\quarantine\..\..\Windows\System32` "starts with" the quarantine root
    /// and walked straight through.
    ///
    /// `take` writes uuid-based names, so nothing reaches `destroy` with a `..`
    /// in it today. This asserts the guard rather than that fact, because the
    /// fact belongs to a different function.
    #[test]
    fn purging_refuses_a_ledger_row_that_climbs_out_of_the_quarantine() {
        let h = harness(RetentionPolicy::default());
        let outside = write(&h.work, "precious.txt", b"do not touch");

        // A row pointing back out through the quarantine root, exactly as the
        // old guard would have accepted it.
        let escape = h
            .quarantine
            .root()
            .join("..")
            .join("work")
            .join("precious.txt");
        assert!(
            escape.starts_with(h.quarantine.root()),
            "the premise: `Path::starts_with` says this is inside the quarantine"
        );

        let item = QuarantineItem {
            id: Uuid::new_v4(),
            batch_id: Uuid::new_v4(),
            original_path: outside.clone(),
            quarantine_path: escape,
            size_bytes: 12,
            quarantined_at: Utc::now() - Duration::days(30),
            purge_after: Utc::now() - Duration::days(1),
            restored_at: None,
            restore_refused_at: None,
        };
        insert_item(&h.conn, &item).expect("insert");

        let report = h
            .quarantine
            .purge_expired(&h.conn, Utc::now())
            .expect("purge");

        assert_eq!(report.purged_items, 0, "nothing outside may be destroyed");
        assert_eq!(report.failed_items, vec![item.id.to_string()]);
        assert!(outside.exists(), "the user's file must survive");
        assert_eq!(fs::read(&outside).expect("read"), b"do not touch");
        assert!(
            h.quarantine
                .find_item(&h.conn, item.id)
                .expect("find")
                .is_some(),
            "the row stays so the refusal is visible rather than silently forgotten"
        );
    }

    /// The same guard, for a row that names a different place entirely.
    #[test]
    fn purging_refuses_a_ledger_row_pointing_outside_the_quarantine_root() {
        let h = harness(RetentionPolicy::default());
        let outside = write(&h.work, "elsewhere.txt", b"not ours");

        let item = QuarantineItem {
            id: Uuid::new_v4(),
            batch_id: Uuid::new_v4(),
            original_path: outside.clone(),
            quarantine_path: outside.clone(),
            size_bytes: 8,
            quarantined_at: Utc::now() - Duration::days(30),
            purge_after: Utc::now() - Duration::days(1),
            restored_at: None,
            restore_refused_at: None,
        };
        insert_item(&h.conn, &item).expect("insert");

        let report = h
            .quarantine
            .purge_expired(&h.conn, Utc::now())
            .expect("purge");
        assert_eq!(report.purged_items, 0);
        assert_eq!(report.failed_items, vec![item.id.to_string()]);
        assert!(outside.exists(), "the user's file must survive");
    }

    /// Security review L8: on the cross-volume branch an occupied destination
    /// used to be flattened into `MoveFailure::Failed`, so `move_out`'s retry
    /// loop never fired and a restore onto a contested name errored out where
    /// the ` (restored)` suffix would have worked.
    #[test]
    fn a_cross_volume_move_reports_an_occupied_destination_as_occupied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = write(dir.path(), "moving.bin", b"the quarantined copy");
        let onto = write(dir.path(), "occupied.bin", b"the user's file");

        fn always_cross_volume(_: &Path, _: &Path) -> std::io::Result<()> {
            #[cfg(windows)]
            const CROSS_VOLUME: i32 = 17;
            #[cfg(not(windows))]
            const CROSS_VOLUME: i32 = 18;
            Err(std::io::Error::from_raw_os_error(CROSS_VOLUME))
        }

        match move_path_with(&source, &onto, always_cross_volume) {
            Err(MoveFailure::Occupied) => {}
            Ok(()) => panic!("the copy must refuse an occupied destination"),
            Err(MoveFailure::Failed(err) | MoveFailure::Encrypted(err)) => panic!(
                "an occupied destination is what the retry loop exists for, \
                 not a failure to report: {err}"
            ),
        }

        assert_eq!(fs::read(&onto).expect("read"), b"the user's file");
        assert_eq!(fs::read(&source).expect("read"), b"the quarantined copy");
    }

    #[test]
    fn default_policy_matches_the_architecture() {
        let policy = RetentionPolicy::default();
        assert_eq!(policy.retention_days, 7);
        assert_eq!(policy.max_bytes, DEFAULT_MAX_BYTES);
    }

    #[test]
    fn quarantine_root_lives_under_the_app_data_dir() {
        let q = Quarantine::new(Path::new("C:\\data"), RetentionPolicy::default());
        assert_eq!(q.root(), Path::new("C:\\data\\quarantine"));
    }

    #[test]
    fn taking_a_file_moves_it_and_writes_a_ledger_row() {
        let h = harness(RetentionPolicy::default());
        let source = write(&h.work, "junk.log", b"0123456789");
        let batch = h.quarantine.start_batch();

        let item = h
            .quarantine
            .take(&h.conn, batch, &source)
            .expect("quarantine the file");

        assert!(!source.exists(), "the original must be gone");
        assert!(item.quarantine_path.is_file(), "the copy must exist");
        assert_eq!(item.size_bytes, 10);
        assert_eq!(item.original_path, source);
        assert!(item.is_held());
        assert!(
            item.quarantine_path.starts_with(h.quarantine.root()),
            "items must live under the quarantine root"
        );

        let stats = h.quarantine.stats(&h.conn).expect("stats");
        assert_eq!(stats.item_count, 1);
        assert_eq!(stats.total_bytes, 10);
    }

    #[test]
    fn the_quarantined_copy_keeps_the_original_extension() {
        let h = harness(RetentionPolicy::default());
        let source = write(&h.work, "report.tmp", b"x");
        let batch = h.quarantine.start_batch();

        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        assert_eq!(
            item.quarantine_path.extension().and_then(|e| e.to_str()),
            Some("tmp")
        );
    }

    /// The acceptance test from the phase brief: clean, then restore, and the
    /// bytes must be exactly what they were.
    #[test]
    fn round_trip_restores_byte_identical_content() {
        let h = harness(RetentionPolicy::default());
        let payload: Vec<u8> = (0..=255u8).cycle().take(300_000).collect();
        let source = write(&h.work, "big.bin", &payload);
        let batch = h.quarantine.start_batch();

        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        assert!(!source.exists());

        let outcome = h
            .quarantine
            .restore_item(&h.conn, item.id)
            .expect("restore");

        assert!(!outcome.renamed);
        assert_eq!(outcome.restored_to, source);
        assert_eq!(fs::read(&source).expect("read back"), payload);
        assert!(
            !item.quarantine_path.exists(),
            "the quarantine copy must be gone after a restore"
        );
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 0);
    }

    #[test]
    fn a_restored_item_cannot_be_restored_twice() {
        let h = harness(RetentionPolicy::default());
        let source = write(&h.work, "once.txt", b"a");
        let batch = h.quarantine.start_batch();
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");

        h.quarantine.restore_item(&h.conn, item.id).expect("first");
        let err = h
            .quarantine
            .restore_item(&h.conn, item.id)
            .expect_err("second restore must fail");
        assert!(matches!(err, CoreError::UnknownQuarantineItem(_)));
    }

    #[test]
    fn restoring_onto_an_occupied_path_never_overwrites() {
        let h = harness(RetentionPolicy::default());
        let source = write(&h.work, "notes.txt", b"original");
        let batch = h.quarantine.start_batch();
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");

        // Something new appeared at the old path while the item was away.
        write(&h.work, "notes.txt", b"newer content");

        let outcome = h
            .quarantine
            .restore_item(&h.conn, item.id)
            .expect("restore");

        assert!(outcome.renamed);
        assert_eq!(outcome.restored_to, h.work.join("notes (restored).txt"));
        assert_eq!(fs::read(&source).expect("read"), b"newer content");
        assert_eq!(fs::read(&outcome.restored_to).expect("read"), b"original");
    }

    #[test]
    fn restoring_recreates_a_deleted_parent_directory() {
        let h = harness(RetentionPolicy::default());
        let source = write(&h.work, "nested/deep/file.txt", b"payload");
        let batch = h.quarantine.start_batch();
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");

        #[allow(clippy::disallowed_methods)] // tearing down this test's own tempdir
        fs::remove_dir_all(h.work.join("nested")).expect("remove the parents");

        let outcome = h
            .quarantine
            .restore_item(&h.conn, item.id)
            .expect("restore");
        assert_eq!(outcome.restored_to, source);
        assert_eq!(fs::read(&source).expect("read"), b"payload");
    }

    #[test]
    fn a_whole_batch_restores_at_once() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let sources: Vec<_> = (0..5)
            .map(|i| {
                write(
                    &h.work,
                    &format!("f{i}.log"),
                    format!("body {i}").as_bytes(),
                )
            })
            .collect();
        for source in &sources {
            h.quarantine.take(&h.conn, batch, source).expect("take");
        }
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 5);

        let (restored, failed) = h
            .quarantine
            .restore_batch(&h.conn, batch)
            .expect("restore batch");

        assert_eq!(restored.len(), 5);
        assert!(failed.is_empty());
        for (i, source) in sources.iter().enumerate() {
            assert_eq!(
                fs::read(source).expect("read"),
                format!("body {i}").into_bytes()
            );
        }
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 0);
    }

    #[test]
    fn directories_round_trip_with_their_whole_tree() {
        let h = harness(RetentionPolicy::default());
        let tree = h.work.join("cache");
        write(&tree, "a.dat", b"aaa");
        write(&tree, "sub/b.dat", b"bbbb");
        let batch = h.quarantine.start_batch();

        let item = h.quarantine.take(&h.conn, batch, &tree).expect("take");
        assert!(!tree.exists());
        assert_eq!(item.size_bytes, 7);

        h.quarantine
            .restore_item(&h.conn, item.id)
            .expect("restore");
        assert_eq!(fs::read(tree.join("a.dat")).expect("read"), b"aaa");
        assert_eq!(fs::read(tree.join("sub/b.dat")).expect("read"), b"bbbb");
    }

    #[test]
    fn symlinks_are_never_quarantined() {
        let h = harness(RetentionPolicy::default());
        let target = write(&h.work, "real.txt", b"data");
        let link = h.work.join("link.txt");

        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&target, &link).is_ok();
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&target, &link).is_ok();

        if !made {
            // M11: loud, not silent. This used to `return` and pass.
            crate::core::test_support::privileged_operation_unavailable(
                "symlinks_are_never_quarantined",
                "creating a file symlink needs Developer Mode or elevation",
            );
            return;
        }

        let batch = h.quarantine.start_batch();
        let err = h
            .quarantine
            .take(&h.conn, batch, &link)
            .expect_err("links must be refused");
        assert!(matches!(err, CoreError::Quarantine(_)));
        assert!(link.exists(), "the link must be left alone");
        assert!(target.exists(), "the target must be untouched");
    }

    /// The same refusal for a **junction**, which any standard user can create —
    /// so this asserts on every developer machine rather than only in CI
    /// (`docs/DECISIONS-NEEDED.md`, architect resolution before phase 2).
    ///
    /// Moving a junction moves the door, not the room: the tree it points at
    /// would be left dangling and the quarantine would hold something it cannot
    /// put back.
    #[cfg(windows)]
    #[test]
    fn junctions_are_never_quarantined() {
        let h = harness(RetentionPolicy::default());
        let target = h.work.join("real-tree");
        fs::create_dir_all(&target).expect("dir");
        write(&target, "payload.txt", b"data");

        let link = h.work.join("door");
        assert!(
            crate::core::test_support::make_junction(&link, &target),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );
        assert!(
            fs::symlink_metadata(&link)
                .expect("the junction exists")
                .is_symlink(),
            "the premise: Windows reports a junction as a link"
        );

        let batch = h.quarantine.start_batch();
        let err = h
            .quarantine
            .take(&h.conn, batch, &link)
            .expect_err("junctions must be refused");
        assert!(matches!(err, CoreError::Quarantine(_)), "got {err}");
        assert!(link.exists(), "the junction must be left alone");
        assert!(
            target.join("payload.txt").exists(),
            "the tree behind it must be untouched"
        );
    }

    #[test]
    fn taking_a_missing_path_leaves_no_ledger_row() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let err = h
            .quarantine
            .take(&h.conn, batch, &h.work.join("ghost.txt"))
            .expect_err("a missing path must fail");
        assert!(matches!(err, CoreError::Quarantine(_)));
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 0);
    }

    #[test]
    fn retention_purge_removes_only_expired_items() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let fresh = write(&h.work, "fresh.log", b"fresh");
        let stale = write(&h.work, "stale.log", b"stale");
        let fresh_item = h.quarantine.take(&h.conn, batch, &fresh).expect("take");
        let stale_item = h.quarantine.take(&h.conn, batch, &stale).expect("take");

        // Backdate one row past its retention window.
        h.conn
            .execute(
                "UPDATE quarantine SET purge_after = ?2 WHERE id = ?1",
                rusqlite::params![
                    stale_item.id.to_string(),
                    (Utc::now() - Duration::days(1)).to_rfc3339()
                ],
            )
            .expect("backdate");

        let report = h
            .quarantine
            .purge_expired(&h.conn, Utc::now())
            .expect("purge");

        assert_eq!(report.purged_items, 1);
        assert_eq!(report.freed_bytes, 5);
        assert!(report.failed_items.is_empty());
        assert!(!stale_item.quarantine_path.exists());
        assert!(fresh_item.quarantine_path.exists());
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 1);
    }

    #[test]
    fn nothing_is_purged_before_its_time() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "recent.log", b"x");
        h.quarantine.take(&h.conn, batch, &source).expect("take");

        let report = h
            .quarantine
            .purge_expired(&h.conn, Utc::now())
            .expect("purge");
        assert!(report.is_empty());
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 1);
    }

    #[test]
    fn the_size_cap_purges_oldest_first() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 25,
        });
        let batch = h.quarantine.start_batch();
        let mut items = Vec::new();
        for i in 0..4 {
            let source = write(&h.work, &format!("f{i}.bin"), &[b'x'; 10]);
            let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
            // Force a deterministic age ordering; the wall clock is too coarse.
            h.conn
                .execute(
                    "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
                    rusqlite::params![
                        item.id.to_string(),
                        (Utc::now() - Duration::minutes(10 - i)).to_rfc3339()
                    ],
                )
                .expect("age");
            items.push(item);
        }
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").total_bytes, 40);

        let report = h.quarantine.enforce_size_cap(&h.conn).expect("cap");

        // 40 held, cap 25: 15 bytes over, so the two oldest 10-byte items go.
        assert_eq!(report.purged_items, 2);
        assert_eq!(report.freed_bytes, 20);
        assert!(!items[0].quarantine_path.exists());
        assert!(!items[1].quarantine_path.exists());
        assert!(items[2].quarantine_path.exists());
        assert!(items[3].quarantine_path.exists());
        assert!(h.quarantine.stats(&h.conn).expect("stats").total_bytes <= 25);
    }

    #[test]
    fn the_size_cap_does_nothing_when_under_the_limit() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 1_000,
        });
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "small.bin", b"tiny");
        h.quarantine.take(&h.conn, batch, &source).expect("take");

        assert!(h
            .quarantine
            .enforce_size_cap(&h.conn)
            .expect("cap")
            .is_empty());
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 1);
    }

    #[test]
    fn purge_runs_retention_and_the_cap_together() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 5,
        });
        let batch = h.quarantine.start_batch();
        let a = write(&h.work, "a.bin", &[b'a'; 10]);
        let b = write(&h.work, "b.bin", &[b'b'; 10]);
        let item_a = h.quarantine.take(&h.conn, batch, &a).expect("take");
        h.quarantine.take(&h.conn, batch, &b).expect("take");
        h.conn
            .execute(
                "UPDATE quarantine SET purge_after = ?2 WHERE id = ?1",
                rusqlite::params![
                    item_a.id.to_string(),
                    (Utc::now() - Duration::days(1)).to_rfc3339()
                ],
            )
            .expect("backdate");

        let report = h.quarantine.purge(&h.conn, Utc::now()).expect("purge");

        assert_eq!(report.purged_items, 2, "expired one plus one over the cap");
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 0);
    }

    #[test]
    fn a_purged_item_leaves_no_ledger_row() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "gone.log", b"x");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        h.conn
            .execute(
                "UPDATE quarantine SET purge_after = ?2 WHERE id = ?1",
                rusqlite::params![
                    item.id.to_string(),
                    (Utc::now() - Duration::days(1)).to_rfc3339()
                ],
            )
            .expect("backdate");

        h.quarantine
            .purge_expired(&h.conn, Utc::now())
            .expect("purge");

        assert_eq!(
            h.quarantine.find_item(&h.conn, item.id).expect("find"),
            None
        );
    }

    #[test]
    fn purge_refuses_paths_outside_the_quarantine_root() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "victim.txt", b"precious");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");

        // Simulate a tampered ledger pointing at a user file.
        let bystander = write(&h.work, "bystander.txt", b"do not touch");
        h.conn
            .execute(
                "UPDATE quarantine SET quarantine_path = ?2, purge_after = ?3 WHERE id = ?1",
                rusqlite::params![
                    item.id.to_string(),
                    bystander.to_string_lossy(),
                    (Utc::now() - Duration::days(1)).to_rfc3339()
                ],
            )
            .expect("tamper");

        let report = h
            .quarantine
            .purge_expired(&h.conn, Utc::now())
            .expect("purge");

        assert_eq!(report.purged_items, 0);
        assert_eq!(report.failed_items, vec![item.id.to_string()]);
        assert!(bystander.exists(), "a path outside the root must survive");
    }

    #[test]
    fn batches_are_listed_newest_first_with_their_totals() {
        let h = harness(RetentionPolicy::default());
        let first = h.quarantine.start_batch();
        let second = h.quarantine.start_batch();
        let a = write(&h.work, "a.log", &[b'a'; 10]);
        let b = write(&h.work, "b.log", &[b'b'; 20]);
        let c = write(&h.work, "c.log", &[b'c'; 30]);
        h.quarantine.take(&h.conn, first, &a).expect("take");
        h.quarantine.take(&h.conn, first, &b).expect("take");
        let c_item = h.quarantine.take(&h.conn, second, &c).expect("take");
        h.conn
            .execute(
                "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
                rusqlite::params![
                    c_item.id.to_string(),
                    (Utc::now() + Duration::minutes(5)).to_rfc3339()
                ],
            )
            .expect("age");

        let batches = h.quarantine.list_batches(&h.conn).expect("batches");

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].batch_id, second);
        assert_eq!(batches[0].item_count, 1);
        assert_eq!(batches[0].size_bytes, 30);
        assert_eq!(batches[1].batch_id, first);
        assert_eq!(batches[1].item_count, 2);
        assert_eq!(batches[1].size_bytes, 30);
    }

    #[test]
    fn restored_items_leave_the_drawer() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "x.log", b"x");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        assert_eq!(
            h.quarantine.list_batches(&h.conn).expect("batches").len(),
            1
        );

        h.quarantine
            .restore_item(&h.conn, item.id)
            .expect("restore");

        assert!(h
            .quarantine
            .list_batches(&h.conn)
            .expect("batches")
            .is_empty());
        assert!(h.quarantine.list_items(&h.conn).expect("items").is_empty());
        let row = h
            .quarantine
            .find_item(&h.conn, item.id)
            .expect("find")
            .expect("the audit row stays");
        assert!(row.restored_at.is_some(), "restores keep an audit trail");
    }

    #[test]
    fn restoring_an_unknown_id_is_an_error() {
        let h = harness(RetentionPolicy::default());
        let err = h
            .quarantine
            .restore_item(&h.conn, Uuid::new_v4())
            .expect_err("unknown id");
        assert!(matches!(err, CoreError::UnknownQuarantineItem(_)));
    }

    #[test]
    fn byte_comparison_notices_a_single_flipped_byte() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = write(dir.path(), "a.bin", &vec![7u8; 200_000]);
        let b = write(dir.path(), "b.bin", &vec![7u8; 200_000]);
        assert!(files_are_identical(&a, &b).expect("compare"));

        let mut bytes = vec![7u8; 200_000];
        bytes[199_999] = 8;
        fs::write(&b, &bytes).expect("rewrite");
        assert!(!files_are_identical(&a, &b).expect("compare"));
    }

    #[test]
    fn byte_comparison_notices_a_length_difference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = write(dir.path(), "a.bin", b"12345");
        let b = write(dir.path(), "b.bin", b"1234");
        assert!(!files_are_identical(&a, &b).expect("compare"));
    }

    #[test]
    fn empty_files_compare_equal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = write(dir.path(), "a.bin", b"");
        let b = write(dir.path(), "b.bin", b"");
        assert!(files_are_identical(&a, &b).expect("compare"));
    }

    #[test]
    fn the_cross_volume_path_copies_verifies_and_removes() {
        // `copy_verify_remove` is what runs when source and destination are on
        // different volumes. That cannot be forced in a tempdir, so the routine
        // itself is exercised directly.
        let dir = tempfile::tempdir().expect("tempdir");
        let source = write(dir.path(), "src.bin", &vec![3u8; 150_000]);
        let target = dir.path().join("dst.bin");

        copy_verify_remove(&source, &target).expect("copy across volumes");

        assert!(!source.exists());
        assert_eq!(fs::read(&target).expect("read").len(), 150_000);
    }

    #[test]
    fn a_failed_verification_leaves_the_original_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = write(dir.path(), "src.bin", b"payload");
        // A directory where a file should go makes the copy fail.
        let target = dir.path().join("dst");
        fs::create_dir(&target).expect("dir");

        let err = copy_verify_remove(&source, &target).expect_err("copy must fail");
        assert!(matches!(
            err,
            CoreError::Quarantine(_) | CoreError::VerifyFailed(_)
        ));
        assert!(source.exists(), "the original must survive a failed move");
    }

    /// The first name a restore would try, mirroring the loop in `move_out`.
    fn first_free_restore_name(original: &Path) -> (PathBuf, bool) {
        for attempt in 0..=RESTORE_ATTEMPTS {
            let (candidate, renamed) = restore_candidate(original, attempt);
            if !is_occupied(&candidate) {
                return (candidate, renamed);
            }
        }
        panic!("no free restore name");
    }

    #[test]
    fn collision_naming_walks_up_until_it_finds_a_free_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = write(dir.path(), "doc.txt", b"a");
        write(dir.path(), "doc (restored).txt", b"b");

        let (target, renamed) = first_free_restore_name(&original);
        assert!(renamed);
        assert_eq!(target, dir.path().join("doc (restored) 2.txt"));
    }

    #[test]
    fn collision_naming_handles_a_file_without_an_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = write(dir.path(), "LICENSE", b"a");
        let (target, renamed) = first_free_restore_name(&original);
        assert!(renamed);
        assert_eq!(target, dir.path().join("LICENSE (restored)"));
    }

    #[test]
    fn restore_names_are_generated_in_a_stable_order() {
        let original = Path::new("C:\\work\\doc.txt");
        let names: Vec<_> = (0..4)
            .map(|attempt| restore_candidate(original, attempt))
            .collect();
        assert_eq!(names[0], (PathBuf::from("C:\\work\\doc.txt"), false));
        assert_eq!(
            names[1],
            (PathBuf::from("C:\\work\\doc (restored).txt"), true)
        );
        assert_eq!(
            names[2],
            (PathBuf::from("C:\\work\\doc (restored) 2.txt"), true)
        );
        assert_eq!(
            names[3],
            (PathBuf::from("C:\\work\\doc (restored) 3.txt"), true)
        );
    }

    #[test]
    fn wire_formats_match_typescript() {
        let json = serde_json::to_string(&QuarantineStats {
            item_count: 2,
            total_bytes: 9,
            unreadable_rows: 0,
            refused_items: 1,
            refused_bytes: 4,
            cap_bytes: 8,
        })
        .expect("serialize");
        assert_eq!(
            json,
            r#"{"itemCount":2,"totalBytes":9,"unreadableRows":0,"refusedItems":1,"refusedBytes":4,"capBytes":8}"#
        );

        let json = serde_json::to_value(PurgeReport {
            purged_items: 1,
            freed_bytes: 2,
            failed_items: Vec::new(),
            affected_batches: vec![Uuid::nil().to_string()],
        })
        .expect("serialize");
        assert!(json.get("affectedBatches").is_some(), "got {json}");

        let json = serde_json::to_value(QuarantineBatch {
            batch_id: Uuid::nil(),
            item_count: 1,
            size_bytes: 2,
            quarantined_at: Utc::now(),
            purge_after: Utc::now(),
        })
        .expect("serialize");
        assert!(json.get("batchId").is_some(), "got {json}");
        assert!(json.get("purgeAfter").is_some(), "got {json}");

        // Follow-up A: warnings travel as `[{path, reason}]`, and a restore
        // without any keeps the JSON it always had.
        let mut outcome = RestoreOutcome {
            item_id: Uuid::nil(),
            restored_to: PathBuf::from("C:\\work\\tree"),
            renamed: false,
            warnings: Vec::new(),
        };
        let json = serde_json::to_value(&outcome).expect("serialize");
        assert!(json.get("warnings").is_none(), "got {json}");
        outcome.warnings.push(MoveWarning {
            path: "C:\\q\\b.txt".to_owned(),
            reason: LEFT_FILE.to_owned(),
        });
        let json = serde_json::to_value(&outcome).expect("serialize");
        assert_eq!(
            json["warnings"],
            serde_json::json!([{ "path": "C:\\q\\b.txt", "reason": LEFT_FILE }])
        );
    }

    /// Round-2 review: a warning about a name that is not valid Unicode must
    /// still reach the UI. A `PathBuf` refuses to serialize there, which would
    /// fail the whole result; the warning carries the display path instead.
    #[cfg(windows)]
    #[test]
    fn a_warning_about_a_non_unicode_name_still_serializes() {
        use std::os::windows::ffi::OsStringExt;
        let lone_surrogate = std::ffi::OsString::from_wide(&[0x61, 0xD800, 0x62]);
        let path = Path::new("C:\\q").join(lone_surrogate);
        let mut report = MoveReport::default();
        report.warn(&path, LEFT_FILE);

        let json = serde_json::to_value(&report.warnings).expect("a lossy path still serializes");
        assert_eq!(json[0]["path"], path.display().to_string());
        assert_eq!(json[0]["reason"], LEFT_FILE);
    }

    /* ---------------------------------------------------------------------
     * Security-review regressions
     * ------------------------------------------------------------------ */

    /// H7: a stored `0` used to expire everything the moment it arrived, and
    /// the startup purge would then empty the drawer before the user saw it.
    #[test]
    fn a_retention_policy_out_of_range_is_clamped() {
        let cases: &[(i64, u64, i64, u64)] = &[
            (0, 0, MIN_RETENTION_DAYS, MIN_MAX_BYTES),
            (-30, 1, MIN_RETENTION_DAYS, MIN_MAX_BYTES),
            (100_000, u64::MAX, MAX_RETENTION_DAYS, u64::MAX),
            (7, DEFAULT_MAX_BYTES, 7, DEFAULT_MAX_BYTES),
        ];
        for (days, bytes, expect_days, expect_bytes) in cases {
            let policy = RetentionPolicy::validated(*days, *bytes);
            assert_eq!(policy.retention_days, *expect_days, "days from {days}");
            assert_eq!(policy.max_bytes, *expect_bytes, "bytes from {bytes}");
        }
        assert_eq!(
            RetentionPolicy::validated(0, 0),
            RetentionPolicy::validated(MIN_RETENTION_DAYS, MIN_MAX_BYTES),
            "clamping is idempotent"
        );
    }

    /// H7, through the constructor the app actually uses.
    #[test]
    fn the_app_constructor_refuses_a_destructive_policy() {
        let quarantine = Quarantine::new(
            Path::new("C:\\data"),
            RetentionPolicy {
                retention_days: 0,
                max_bytes: 0,
            },
        );
        assert_eq!(quarantine.policy().retention_days, MIN_RETENTION_DAYS);
        assert_eq!(quarantine.policy().max_bytes, MIN_MAX_BYTES);
    }

    /// H1: a failed copy used to `remove_dir_all` its destination without
    /// asking whether it had created it. On the restore path that destination
    /// is a user directory.
    #[test]
    fn a_failed_copy_never_removes_a_destination_it_did_not_create() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = write(dir.path(), "src.bin", b"payload");

        // The user's own folder, sitting where the restore wants to land.
        let occupied = dir.path().join("dst");
        fs::create_dir(&occupied).expect("dir");
        let precious = write(&occupied, "precious.txt", b"do not touch");

        let err = copy_verify_remove(&source, &occupied).expect_err("the copy must fail");

        assert!(matches!(
            err,
            CoreError::Quarantine(_) | CoreError::VerifyFailed(_)
        ));
        assert!(occupied.is_dir(), "the user's directory must survive");
        assert_eq!(fs::read(&precious).expect("read"), b"do not touch");
        assert!(source.exists(), "the original must survive too");
    }

    /// M10: H1's fix sampled the destination with a probe *before* the copy,
    /// which answers a different question. A file that appears at `to` between
    /// the probe and `create_new` makes the copy fail with `AlreadyExists`
    /// while the probe still says "did not exist" — and on the restore path
    /// `to` is a user path, so the cleanup deleted the file that appeared.
    ///
    /// The fix is that the copy itself reports whether it created `to`, so
    /// these are the two cases that matter, asserted on the copy primitives
    /// directly rather than on a race that cannot be scheduled.
    #[test]
    fn a_copy_reports_whether_it_created_its_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = write(dir.path(), "src.bin", b"payload");

        // Nothing there: the copy creates it and says so.
        let fresh = dir.path().join("fresh.bin");
        let mut created = false;
        copy_file(&source, &fresh, &mut created).expect("copy");
        assert!(created, "a copy that created `to` must say so");

        // Something appeared there first: `create_new` fails and the copy must
        // *not* claim it, whatever a probe taken earlier would have said.
        let squatter = write(dir.path(), "squatter.bin", b"the user's file");
        let mut created = false;
        let err = copy_file(&source, &squatter, &mut created)
            .expect_err("an occupied destination must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(
            !created,
            "a copy that did not create `to` must never claim it"
        );

        // And that is what the cleanup keys on: it leaves what it did not make.
        cleanup_partial(&squatter, created);
        assert_eq!(
            fs::read(&squatter).expect("read"),
            b"the user's file",
            "cleanup must not delete a file this call never created"
        );
        // The one it did make, it removes.
        cleanup_partial(&fresh, true);
        assert!(!fresh.exists());
    }

    /// The directory half of M10: `create_dir` fails on an occupied name, so
    /// the tree copy must not claim a directory it did not create either.
    #[test]
    fn a_directory_copy_reports_whether_it_created_its_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("src");
        fs::create_dir(&source).expect("dir");
        write(&source, "a.bin", b"payload");

        let fresh = dir.path().join("fresh");
        let mut created = false;
        copy_dir_recursive(&source, &fresh, &mut created).expect("copy");
        assert!(created);

        let occupied = dir.path().join("occupied");
        fs::create_dir(&occupied).expect("dir");
        let precious = write(&occupied, "precious.txt", b"do not touch");
        let mut created = false;
        copy_dir_recursive(&source, &occupied, &mut created)
            .expect_err("an occupied destination must fail");
        assert!(!created);

        cleanup_partial(&occupied, created);
        assert_eq!(fs::read(&precious).expect("read"), b"do not touch");
    }

    /// H2: `Path::exists` answers "no" for a path we merely cannot see, and
    /// `fs::rename` replaces. Together that overwrites a file on restore.
    #[test]
    fn a_restore_never_replaces_an_occupied_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let from = write(dir.path(), "moving.bin", b"the quarantined copy");
        let onto = write(dir.path(), "occupied.bin", b"the user's file");

        match move_path(&from, &onto) {
            Err(MoveFailure::Occupied) => {}
            Ok(()) => panic!("the move must refuse an occupied destination"),
            Err(MoveFailure::Failed(err) | MoveFailure::Encrypted(err)) => {
                panic!("wrong failure: {err}")
            }
        }

        assert_eq!(fs::read(&onto).expect("read"), b"the user's file");
        assert_eq!(fs::read(&from).expect("read"), b"the quarantined copy");
    }

    #[test]
    fn occupancy_is_probed_with_symlink_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!is_occupied(&dir.path().join("nothing-here")));
        assert!(is_occupied(&write(dir.path(), "here.txt", b"x")));
        assert!(is_occupied(dir.path()), "a directory occupies its name too");
    }

    /// M2: the row is marked before the move, so a crash cannot leave a row
    /// claiming to hold a file that is already back home.
    #[test]
    fn a_failed_restore_puts_the_ledger_row_back() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "wanted.txt", b"payload");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");

        // The quarantined copy disappears behind our back, so the move fails.
        #[allow(clippy::disallowed_methods)] // simulating a copy that went missing
        fs::remove_file(&item.quarantine_path).expect("remove the copy");

        let err = h
            .quarantine
            .restore_item(&h.conn, item.id)
            .expect_err("the restore must fail");
        assert!(matches!(err, CoreError::Quarantine(_)));

        let row = h
            .quarantine
            .find_item(&h.conn, item.id)
            .expect("find")
            .expect("the row must still be there");
        assert!(
            row.is_held(),
            "a restore that did not happen must leave the row held"
        );
        assert_eq!(h.quarantine.stats(&h.conn).expect("stats").item_count, 1);
    }

    /// M4 and H8 together. A zero-byte item cannot reduce `over_by`, so
    /// evicting it purges the quarantine to reclaim nothing (M4) — but
    /// *stopping* at it disabled the cap outright, because candidates are
    /// ordered oldest-first and one empty `.tmp` at the head left the
    /// quarantine over its cap for the whole retention window (H8). The right
    /// answer is neither: skip it and keep evicting real items.
    #[test]
    fn the_size_cap_skips_zero_byte_items_and_keeps_evicting() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 5,
        });
        let batch = h.quarantine.start_batch();

        let empty = write(&h.work, "empty.bin", b"");
        let real = write(&h.work, "real.bin", &[b'x'; 10]);
        let empty_item = h.quarantine.take(&h.conn, batch, &empty).expect("take");
        let real_item = h.quarantine.take(&h.conn, batch, &real).expect("take");
        // The empty one is the *oldest*, so it is the head of the candidate
        // list — exactly the arrangement that used to abort the eviction.
        for (index, item) in [&empty_item, &real_item].iter().enumerate() {
            h.conn
                .execute(
                    "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
                    rusqlite::params![
                        item.id.to_string(),
                        timestamp(Utc::now() - Duration::minutes(10 - index as i64))
                    ],
                )
                .expect("age");
        }

        let report = h.quarantine.enforce_size_cap(&h.conn).expect("cap");

        assert_eq!(
            report.purged_items, 1,
            "the 10-byte item is what puts the quarantine over its 5-byte cap"
        );
        assert_eq!(report.freed_bytes, 10);
        assert!(
            !real_item.quarantine_path.exists(),
            "the item that was actually over the cap must be evicted"
        );
        assert!(
            empty_item.quarantine_path.exists(),
            "a zero-byte item frees nothing, so evicting it would cost the user a \
             restorable file for no reclaimed bytes"
        );
        assert!(h.quarantine.stats(&h.conn).expect("stats").total_bytes <= 5);
    }

    /// H8, the shape that disabled the cap indefinitely: a steady trickle of
    /// empty `.tmp`/lock files — exactly what this app quarantines — at the
    /// head of the candidate list. The eviction must walk past all of them and
    /// reach the item that is actually holding the bytes.
    #[test]
    fn a_long_run_of_zero_byte_items_does_not_disable_the_size_cap() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 5,
        });
        let batch = h.quarantine.start_batch();

        let mut empties = Vec::new();
        for index in 0..25 {
            let path = write(&h.work, &format!("empty-{index}.tmp"), b"");
            empties.push(h.quarantine.take(&h.conn, batch, &path).expect("take"));
        }
        let big = write(&h.work, "big.bin", &[b'x'; 100]);
        let big_item = h.quarantine.take(&h.conn, batch, &big).expect("take");

        // Every empty is older than the big one, so all 25 are ahead of it.
        for (index, item) in empties.iter().enumerate() {
            h.conn
                .execute(
                    "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
                    rusqlite::params![
                        item.id.to_string(),
                        timestamp(Utc::now() - Duration::minutes(60 - index as i64))
                    ],
                )
                .expect("age");
        }

        let report = h.quarantine.enforce_size_cap(&h.conn).expect("cap");

        assert_eq!(report.purged_items, 1, "only the item holding bytes");
        assert_eq!(report.freed_bytes, 100);
        assert!(!big_item.quarantine_path.exists());
        for item in &empties {
            assert!(
                item.quarantine_path.exists(),
                "zero-byte items are skipped, not evicted"
            );
        }
        assert!(
            h.quarantine.stats(&h.conn).expect("stats").total_bytes <= 5,
            "the cap must actually be enforced"
        );
    }

    /// M4: an unreadable directory used to be measured as zero bytes.
    #[test]
    fn a_directory_that_cannot_be_measured_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = directory_size(&dir.path().join("not-a-directory"))
            .expect_err("an unmeasurable tree must not be reported as empty");
        assert!(matches!(err, CoreError::Quarantine(_)));

        // The happy path still adds up.
        write(dir.path(), "a.bin", &[b'a'; 3]);
        write(dir.path(), "sub/b.bin", &[b'b'; 4]);
        assert_eq!(directory_size(dir.path()).expect("measure"), 7);
    }

    /// M5: the cap crosses batch boundaries, so the report has to say which
    /// batches it took things out of.
    #[test]
    fn a_purge_names_the_batches_it_touched() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 5,
        });
        let first = h.quarantine.start_batch();
        let second = h.quarantine.start_batch();
        let a = write(&h.work, "a.bin", &[b'a'; 10]);
        let b = write(&h.work, "b.bin", &[b'b'; 10]);
        let item_a = h.quarantine.take(&h.conn, first, &a).expect("take");
        h.quarantine.take(&h.conn, second, &b).expect("take");
        h.conn
            .execute(
                "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
                rusqlite::params![
                    item_a.id.to_string(),
                    timestamp(Utc::now() - Duration::minutes(10))
                ],
            )
            .expect("age");

        let report = h.quarantine.enforce_size_cap(&h.conn).expect("cap");

        assert!(report.purged_items >= 1);
        assert!(
            report.affected_batches.contains(&first.to_string()),
            "the batch the user is looking at must be named, got {:?}",
            report.affected_batches
        );
    }

    /// Mark `id` the way a restore refused for EFS does.
    fn mark_refused(h: &Harness, id: Uuid) {
        h.conn
            .execute(
                "UPDATE quarantine SET restore_refused_at = ?2 WHERE id = ?1",
                rusqlite::params![id.to_string(), timestamp(Utc::now())],
            )
            .expect("mark");
    }

    /// Decision b (2026-10-12): an item whose restore was refused because it
    /// is encrypted outlives its retention window; only the user removes it.
    #[test]
    fn the_retention_purge_keeps_an_item_whose_restore_was_refused() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let kept = write(&h.work, "kept.log", b"kept");
        let stale = write(&h.work, "stale.log", b"stale");
        let kept_item = h.quarantine.take(&h.conn, batch, &kept).expect("take");
        let stale_item = h.quarantine.take(&h.conn, batch, &stale).expect("take");
        for id in [kept_item.id, stale_item.id] {
            h.conn
                .execute(
                    "UPDATE quarantine SET purge_after = ?2 WHERE id = ?1",
                    rusqlite::params![id.to_string(), timestamp(Utc::now() - Duration::days(1))],
                )
                .expect("backdate");
        }
        mark_refused(&h, kept_item.id);

        let report = h
            .quarantine
            .purge_expired(&h.conn, Utc::now())
            .expect("purge");

        assert_eq!(report.purged_items, 1, "only the unmarked item expires");
        assert!(!stale_item.quarantine_path.exists());
        assert_eq!(fs::read(&kept_item.quarantine_path).expect("kept"), b"kept");
        let held = h
            .quarantine
            .find_item(&h.conn, kept_item.id)
            .expect("ledger")
            .expect("the row stays");
        assert!(held.is_held());
        assert!(held.restore_refused_at.is_some());
    }

    /// Decision b: the size cap skips a marked item even when it is the
    /// oldest, and evicts the next one instead. Round 4: the marked item is
    /// not counted against the cap either, so only one unmarked item goes.
    #[test]
    fn the_size_cap_skips_an_item_whose_restore_was_refused() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 15,
        });
        let batch = h.quarantine.start_batch();
        let mut items = Vec::new();
        for i in 0..3 {
            let source = write(&h.work, &format!("f{i}.bin"), &[b'x'; 10]);
            let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
            h.conn
                .execute(
                    "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
                    rusqlite::params![
                        item.id.to_string(),
                        timestamp(Utc::now() - Duration::minutes(10 - i))
                    ],
                )
                .expect("age");
            items.push(item);
        }
        mark_refused(&h, items[0].id);

        let report = h.quarantine.enforce_size_cap(&h.conn).expect("cap");

        // 20 unmarked, cap 15: the marked oldest stays, the next one goes,
        // and the newest fits under the cap on its own.
        assert_eq!(report.purged_items, 1);
        assert!(items[0].quarantine_path.exists(), "the marked item is kept");
        assert!(!items[1].quarantine_path.exists());
        assert!(items[2].quarantine_path.exists());
    }

    /// Decision b: when only marked items are left, the cap cannot be met
    /// and nothing is removed.
    #[test]
    fn the_size_cap_removes_nothing_when_only_refused_items_are_over_it() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 5,
        });
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "big.bin", &[b'x'; 10]);
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        mark_refused(&h, item.id);

        let report = h.quarantine.purge(&h.conn, Utc::now()).expect("purge");

        assert!(report.is_empty(), "got {report:?}");
        assert!(item.quarantine_path.exists());
    }

    /// Round 4: the items kept after a refused restore are not counted
    /// against the cap. Round 3 removed nothing for size once they filled the
    /// cap on their own, which left a cliff just under it; now the cap
    /// applies to unmarked bytes only, whatever the marked items hold.
    #[test]
    fn refused_items_are_not_counted_against_the_size_cap() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 15,
        });
        let batch = h.quarantine.start_batch();
        let mut unmarked = Vec::new();
        for i in 0..2 {
            let source = write(&h.work, &format!("old{i}.bin"), &[b'x'; 10]);
            let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
            h.conn
                .execute(
                    "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
                    rusqlite::params![
                        item.id.to_string(),
                        timestamp(Utc::now() - Duration::minutes(10 - i))
                    ],
                )
                .expect("age");
            unmarked.push(item);
        }
        let encrypted = write(&h.work, "encrypted.bin", &[b'x'; 20]);
        let marked = h.quarantine.take(&h.conn, batch, &encrypted).expect("take");
        mark_refused(&h, marked.id);

        let report = h.quarantine.purge(&h.conn, Utc::now()).expect("purge");

        // 20 unmarked, cap 15: the oldest unmarked goes, the marked 20 do not
        // count, and the other unmarked item fits.
        assert_eq!(report.purged_items, 1, "got {report:?}");
        assert!(!unmarked[0].quarantine_path.exists());
        assert!(unmarked[1].quarantine_path.exists());
        assert!(marked.quarantine_path.exists());
        let stats = h.quarantine.stats(&h.conn).expect("stats");
        assert_eq!(stats.refused_items, 1);
        assert_eq!(stats.refused_bytes, 20);
        assert_eq!(stats.cap_bytes, 15);
        assert_eq!(stats.total_bytes, 30);
    }

    /// Round 3: refused items do not stop the cap from evicting the oldest
    /// unmarked items. Round 4: it evicts until the unmarked bytes fit.
    #[test]
    fn the_size_cap_still_evicts_when_refused_items_are_under_it() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 25,
        });
        let batch = h.quarantine.start_batch();
        let mut items = Vec::new();
        for i in 0..4 {
            let source = write(&h.work, &format!("g{i}.bin"), &[b'x'; 10]);
            let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
            h.conn
                .execute(
                    "UPDATE quarantine SET quarantined_at = ?2 WHERE id = ?1",
                    rusqlite::params![
                        item.id.to_string(),
                        timestamp(Utc::now() - Duration::minutes(10 - i))
                    ],
                )
                .expect("age");
            items.push(item);
        }
        mark_refused(&h, items[3].id);

        let report = h.quarantine.enforce_size_cap(&h.conn).expect("cap");

        // 30 unmarked, cap 25: only the oldest unmarked goes.
        assert_eq!(report.purged_items, 1);
        assert!(!items[0].quarantine_path.exists());
        assert!(items[1].quarantine_path.exists());
        assert!(items[2].quarantine_path.exists());
        assert!(items[3].quarantine_path.exists());
        let stats = h.quarantine.stats(&h.conn).expect("stats");
        assert!(
            stats.total_bytes - stats.refused_bytes <= 25,
            "got {stats:?}"
        );
        assert_eq!(stats.refused_bytes, 10);
    }

    /// Round 4: marked items just under the cap used to leave a few bytes
    /// for everything else, so any clean bigger than that evicted older
    /// items at the next start. An unmarked item that fits under the cap on
    /// its own now stays, whatever the marked items hold.
    #[test]
    fn marked_items_just_under_the_cap_do_not_evict_what_fits() {
        let h = harness(RetentionPolicy {
            retention_days: 7,
            max_bytes: 25,
        });
        let batch = h.quarantine.start_batch();
        let encrypted = write(&h.work, "encrypted.bin", &[b'x'; 24]);
        let marked = h.quarantine.take(&h.conn, batch, &encrypted).expect("take");
        mark_refused(&h, marked.id);
        let source = write(&h.work, "fits.bin", &[b'x'; 10]);
        let fits = h.quarantine.take(&h.conn, batch, &source).expect("take");

        let report = h.quarantine.enforce_size_cap(&h.conn).expect("cap");

        assert!(report.is_empty(), "got {report:?}");
        assert!(fits.quarantine_path.exists(), "10 bytes fit under 25");
        assert!(marked.quarantine_path.exists());
    }

    /// Round 3: an id that was already restored is not held, so it cannot be
    /// removed by hand.
    #[test]
    fn removing_a_restored_item_by_hand_is_refused() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "back.bin", b"back");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        mark_refused(&h, item.id);
        h.quarantine
            .restore_item(&h.conn, item.id)
            .expect("restore");

        let err = h
            .quarantine
            .purge_item(&h.conn, item.id)
            .expect_err("a restored item is not held");

        assert!(
            matches!(err, CoreError::UnknownQuarantineItem(_)),
            "got {err}"
        );
        assert_eq!(fs::read(&source).expect("restored"), b"back");
    }

    /// Round 3: a refused restore promises "kept until you remove it" only
    /// when the marker was recorded. Without it the message says the
    /// automatic purge may still remove the item.
    #[test]
    fn a_refused_restore_promises_to_keep_the_item_only_when_marked() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "secret.txt", b"secret");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        let cause = CoreError::Quarantine("is encrypted with Windows EFS".into());

        let kept = encrypted_restore_refused(&item, &cause, true).to_string();
        assert!(kept.contains("until you remove it"), "{kept}");

        let unprotected = encrypted_restore_refused(&item, &cause, false).to_string();
        assert!(
            !unprotected.contains("until you remove it"),
            "{unprotected}"
        );
        assert!(
            unprotected.contains("automatic purge may still remove it"),
            "{unprotected}"
        );
        assert!(unprotected.contains("decrypt"), "{unprotected}");
    }

    /// Round 3: a marker the ledger refuses to record is reported back, so
    /// the restore does not promise what it could not keep.
    #[test]
    fn marking_a_refused_restore_reports_a_failure() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "m.bin", b"m");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        assert!(
            mark_restore_refused(&h.conn, item.id),
            "an unhindered mark works"
        );

        let other = write(&h.work, "n.bin", b"n");
        let blocked = h.quarantine.take(&h.conn, batch, &other).expect("take");
        h.conn
            .execute_batch(
                "CREATE TRIGGER refuse_marker BEFORE UPDATE OF restore_refused_at ON quarantine
                 BEGIN SELECT RAISE(ABORT, 'marker refused'); END;",
            )
            .expect("trigger");

        assert!(!mark_restore_refused(&h.conn, blocked.id));
    }

    /// Round 3: every warning reason reads as one sentence. A string literal
    /// broken over lines without a backslash kept the indentation as runs of
    /// spaces.
    #[test]
    fn warning_reasons_have_no_runs_of_spaces() {
        for reason in [
            LEFT_FILE,
            LEFT_DIR,
            COPY_ONLY,
            STREAM_LOST,
            STREAM_UNCONFIRMED,
            BOTH_KEPT,
        ] {
            assert!(!reason.contains("  "), "{reason:?}");
            assert!(!reason.contains('\n'), "{reason:?}");
        }
    }

    /// Round 4: the same slip in a log message. A line of this file with a
    /// long run of spaces after its first word is a string literal that lost
    /// its backslash continuation.
    #[test]
    fn log_messages_in_this_file_have_no_runs_of_spaces() {
        let run = " ".repeat(8);
        for (number, line) in include_str!("quarantine.rs").lines().enumerate() {
            assert!(
                !line.trim_start().contains(&run),
                "line {}: {line:?}",
                number + 1
            );
        }
    }

    /// Decision b: the user can still remove a marked item explicitly.
    #[test]
    fn the_user_can_remove_an_item_whose_restore_was_refused() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "held.bin", b"held");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");
        mark_refused(&h, item.id);

        let report = h.quarantine.purge_item(&h.conn, item.id).expect("remove");

        assert_eq!(report.purged_items, 1);
        assert_eq!(report.freed_bytes, 4);
        assert!(!item.quarantine_path.exists());
        assert!(h
            .quarantine
            .find_item(&h.conn, item.id)
            .expect("ledger")
            .is_none());
    }

    /// `purge_item` exists for the items the automatic purge will not touch;
    /// it refuses anything else rather than becoming a second delete button.
    #[test]
    fn removing_an_item_by_hand_is_only_for_refused_restores() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let source = write(&h.work, "plain.bin", b"plain");
        let item = h.quarantine.take(&h.conn, batch, &source).expect("take");

        let err = h
            .quarantine
            .purge_item(&h.conn, item.id)
            .expect_err("an unmarked item is left to the retention window");
        assert!(
            matches!(err, CoreError::QuarantineItemNotMarked(_)),
            "got {err}"
        );
        assert_eq!(err.code(), "quarantine_item_not_marked");
        assert!(item.quarantine_path.exists());

        let unknown = h
            .quarantine
            .purge_item(&h.conn, Uuid::new_v4())
            .expect_err("an unknown id");
        assert!(matches!(unknown, CoreError::UnknownQuarantineItem(_)));
    }

    /// M6: a cross-volume copy used to arrive with today's timestamps, so a
    /// restored file looked brand new to every backup and every age rule.
    /// The rename is injected, because a tempdir has only one volume.
    #[test]
    fn a_cross_volume_move_keeps_the_timestamps_and_the_read_only_flag() {
        fn always_cross_volume(_from: &Path, _to: &Path) -> std::io::Result<()> {
            #[cfg(windows)]
            const CROSS_VOLUME: i32 = 17;
            #[cfg(not(windows))]
            const CROSS_VOLUME: i32 = 18;
            Err(std::io::Error::from_raw_os_error(CROSS_VOLUME))
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let source = write(dir.path(), "aged.bin", &[9u8; 4_096]);
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(72 * 3_600);
        fs::File::options()
            .write(true)
            .open(&source)
            .expect("open")
            .set_times(fs::FileTimes::new().set_modified(when).set_accessed(when))
            .expect("backdate");
        let mut permissions = fs::metadata(&source).expect("meta").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&source, permissions).expect("readonly");

        let target = dir.path().join("moved.bin");
        move_path_with(&source, &target, always_cross_volume).unwrap_or_else(|err| {
            panic!(
                "the cross-volume branch must succeed: {}",
                err.into_error(Path::new("a"), Path::new("b"))
            )
        });

        assert!(!source.exists(), "the original moves");
        let moved = fs::metadata(&target).expect("meta");
        assert_eq!(moved.len(), 4_096);
        assert!(moved.permissions().readonly(), "the flag comes across");
        let apart = moved
            .modified()
            .expect("modified")
            .duration_since(when)
            .or_else(|_| when.duration_since(moved.modified().expect("modified")))
            .expect("duration");
        assert!(
            apart <= TIME_TOLERANCE,
            "the modification time must come across, was {apart:?} out"
        );
    }

    /// M6: verification has to notice a copy that lost its metadata, or
    /// preserving it is unenforced.
    #[test]
    fn verification_notices_a_copy_that_lost_its_timestamps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = write(dir.path(), "a.bin", b"same bytes");
        let b = write(dir.path(), "b.bin", b"same bytes");

        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(48 * 3_600);
        fs::File::options()
            .write(true)
            .open(&a)
            .expect("open")
            .set_times(fs::FileTimes::new().set_modified(when).set_accessed(when))
            .expect("backdate");

        assert!(
            files_are_identical(&a, &b).expect("compare"),
            "the bytes are the same"
        );
        assert!(
            !copies_match(&a, &b).expect("compare"),
            "but the copy is not faithful"
        );
    }

    /// M7: a link inside a quarantined tree used to be dereferenced — stored as
    /// a real file and destroyed on the way back. The move is refused instead,
    /// and the original is left exactly where it is.
    #[test]
    fn a_directory_containing_a_link_is_never_moved_across_volumes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        let payload = write(&tree, "real.txt", b"the data");
        let link = tree.join("link.txt");

        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&payload, &link).is_ok();
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&payload, &link).is_ok();
        if !made {
            // M11: loud, not silent. `copy_dir_recursive`'s link handling is
            // additionally covered without privileges by
            // `the_copy_reports_links_instead_of_following_them` below.
            crate::core::test_support::privileged_operation_unavailable(
                "a_directory_containing_a_link_is_never_moved_across_volumes",
                "creating a file symlink needs Developer Mode or elevation",
            );
            return;
        }

        let target = dir.path().join("copied");
        let err = copy_verify_remove(&tree, &target).expect_err("links must refuse the move");

        assert!(matches!(err, CoreError::Quarantine(_)));
        assert!(
            err.to_string().contains("link"),
            "the refusal must say why: {err}"
        );
        assert!(tree.is_dir(), "the original tree must survive");
        assert!(link.symlink_metadata().is_ok(), "the link must survive");
        assert_eq!(fs::read(&payload).expect("read"), b"the data");
        assert!(!target.exists(), "the half-copy is cleaned up");
    }

    /// The same refusal when the link inside the tree is a **junction**, which
    /// needs no privilege — so this asserts on every developer machine rather
    /// than only in CI (`docs/DECISIONS-NEEDED.md`, architect resolution before
    /// phase 2).
    #[cfg(windows)]
    #[test]
    fn a_directory_containing_a_junction_is_never_moved_across_volumes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        let payload = write(&tree, "real.txt", b"the data");
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).expect("dir");

        let link = tree.join("door");
        assert!(
            crate::core::test_support::make_junction(&link, &elsewhere),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );

        let target = dir.path().join("copied");
        let err = copy_verify_remove(&tree, &target).expect_err("junctions must refuse the move");

        assert!(matches!(err, CoreError::Quarantine(_)), "got {err}");
        assert!(
            err.to_string().contains("link"),
            "the refusal must say why: {err}"
        );
        assert!(tree.is_dir(), "the original tree must survive");
        assert!(link.exists(), "the junction must survive");
        assert_eq!(fs::read(&payload).expect("read"), b"the data");
    }

    /// M7 without needing link privileges: the copy reports links rather than
    /// following them, and the comparison distinguishes a link from a file.
    #[test]
    fn the_copy_reports_links_instead_of_following_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        write(&tree, "a.txt", b"a");
        write(&tree, "sub/b.txt", b"b");

        let target = dir.path().join("copy");
        let skipped = copy_dir_recursive(&tree, &target, &mut false).expect("copy");

        assert!(skipped.is_empty(), "a plain tree skips nothing");
        assert!(dirs_are_identical(&tree, &target).expect("compare"));

        // A file where the source has a directory is not the same tree.
        let mismatched = dir.path().join("mismatched");
        fs::create_dir(&mismatched).expect("dir");
        write(&mismatched, "a.txt", b"a");
        write(&mismatched, "sub", b"not a directory");
        assert!(!dirs_are_identical(&tree, &mismatched).expect("compare"));
    }

    /// L1: on Windows the cross-volume errno is 17; 18 is ERROR_NO_MORE_FILES
    /// and used to start a copy.
    #[test]
    fn only_the_real_cross_volume_errno_starts_a_copy() {
        let cross = if cfg!(windows) { 17 } else { 18 };
        let other = if cfg!(windows) { 18 } else { 17 };
        assert!(is_cross_volume(&std::io::Error::from_raw_os_error(cross)));
        assert!(!is_cross_volume(&std::io::Error::from_raw_os_error(other)));
        assert!(!is_cross_volume(&std::io::Error::from_raw_os_error(32)));
        assert!(!is_cross_volume(&std::io::Error::other("no errno at all")));
    }

    /// L2: the retention query compares timestamps as text, so they have to
    /// sort the way they read.
    #[test]
    fn timestamps_sort_lexicographically() {
        let base = DateTime::parse_from_rfc3339("2026-08-06T09:00:00Z")
            .expect("parse")
            .with_timezone(&Utc);
        let instants = [
            base,
            base + Duration::milliseconds(1),
            base + Duration::seconds(1),
            base + Duration::days(1),
            base + Duration::days(400),
        ];

        let written: Vec<String> = instants.iter().copied().map(timestamp).collect();
        let mut sorted = written.clone();
        sorted.sort();
        assert_eq!(written, sorted, "text order must match time order");

        for (instant, text) in instants.iter().zip(&written) {
            assert!(text.ends_with('Z'), "always UTC: {text}");
            assert_eq!(
                text.len(),
                written[0].len(),
                "always the same width: {text}"
            );
            assert_eq!(
                parse_time(text).expect("round trip").timestamp_millis(),
                instant.timestamp_millis()
            );
        }
        // Rows written by the previous format still read back.
        assert!(parse_time("2026-08-06T09:00:00+00:00").is_some());
    }

    /// L4: a corrupt row used to become a nil-uuid item dated 1970 that the
    /// drawer offered to restore and the purge thought was long expired.
    #[test]
    fn corrupt_ledger_rows_are_skipped_and_counted() {
        let h = harness(RetentionPolicy::default());
        let batch = h.quarantine.start_batch();
        let good = write(&h.work, "good.bin", &[b'g'; 10]);
        let bad = write(&h.work, "bad.bin", &[b'b'; 10]);
        h.quarantine.take(&h.conn, batch, &good).expect("take");
        let bad_item = h.quarantine.take(&h.conn, batch, &bad).expect("take");

        h.conn
            .execute(
                "UPDATE quarantine SET quarantined_at = 'not a timestamp' WHERE id = ?1",
                [bad_item.id.to_string()],
            )
            .expect("corrupt the row");

        let stats = h.quarantine.stats(&h.conn).expect("stats");
        assert_eq!(stats.item_count, 1, "only the readable row counts");
        assert_eq!(stats.total_bytes, 10);
        assert_eq!(stats.unreadable_rows, 1, "and the other one is reported");

        assert_eq!(h.quarantine.list_items(&h.conn).expect("items").len(), 1);
        assert_eq!(
            h.quarantine.find_item(&h.conn, bad_item.id).expect("find"),
            None,
            "an unparsable row is not an item"
        );

        // A purge cannot mistake it for something long expired.
        let report = h
            .quarantine
            .purge_expired(&h.conn, Utc::now())
            .expect("purge");
        assert!(report.is_empty(), "got {report:?}");
        assert!(bad_item.quarantine_path.exists());
    }

    /// CLAUDE.md hard rule. The *enforcement* is `clippy.toml`, which resolves
    /// the path of every call and so cannot be walked past with an aliased
    /// import or a fully-qualified spelling. This is the smoke check that goes
    /// with it: it reads the whole crate — `src/`, `tests/` and `build.rs`, not
    /// just `src/` as it used to — and catches the same thing one commit
    /// earlier, at `cargo test` rather than `cargo clippy`
    /// (security review M1).
    #[test]
    fn no_other_module_removes_user_data() {
        /// Spellings of a permanent removal. The lint covers the resolved
        /// path; these cover the text, including the fully-qualified forms the
        /// previous version of this test missed.
        const FORBIDDEN: &[&str] = &[
            "fs::remove_file",
            "fs::remove_dir",
            "remove_dir_all",
            "std::fs::remove",
            "DeleteFileW",
            "RemoveDirectoryW",
            // Deleting through an open handle, which is how this module
            // removes a source file it has pinned.
            "FileDispositionInfo",
            "SetFileInformationByHandle",
            "NtSetInformationFile",
            // Delete when the last handle closes, by name and by value
            // (adversarial review, LOW 4).
            "FILE_FLAG_DELETE_ON_CLOSE",
            "0x04000000",
            "0x0400_0000",
            // The ANSI spellings and the shell's own removal.
            "DeleteFileA",
            "RemoveDirectoryA",
            "SHFileOperation",
            "IFileOperation",
            // A move that replaces whatever is at the destination destroys it.
            "MOVEFILE_REPLACE_EXISTING",
            // The move call itself: only this module moves user data, and a
            // move can be asked to replace (adversarial review follow-up E).
            "MoveFileEx",
        ];
        /// The handle openers made for removing: only this module may call
        /// them, and `core/platform/pinned.rs` defines them.
        const DELETE_OPENERS: &[&str] = &["open_to_delete", "open_dir_to_delete"];

        fn visit(path: &Path, sanctioned: &[PathBuf], openers: &Path, offenders: &mut Vec<String>) {
            if path.is_dir() {
                let Ok(entries) = fs::read_dir(path) else {
                    return;
                };
                for entry in entries.flatten() {
                    visit(&entry.path(), sanctioned, openers, offenders);
                }
                return;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                return;
            }
            // This very file is the sanctioned exception, by its full path:
            // any other file that happens to be called `quarantine.rs` is not.
            if sanctioned.iter().any(|exempt| exempt == path) {
                return;
            }
            let Ok(source) = fs::read_to_string(path) else {
                return;
            };
            let defines_openers = path == openers;
            for (number, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                let removes = FORBIDDEN.iter().any(|needle| line.contains(needle));
                let opens_to_remove =
                    !defines_openers && DELETE_OPENERS.iter().any(|needle| line.contains(needle));
                if removes || opens_to_remove {
                    offenders.push(format!("{}:{}", path.display(), number + 1));
                }
            }
        }

        // Read-only, and bounded to this crate's own sources.
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let sanctioned = [crate_root.join("src").join("core").join("quarantine.rs")];
        let openers = crate_root
            .join("src")
            .join("core")
            .join("platform")
            .join("pinned.rs");
        let mut offenders = Vec::new();
        for target in ["src", "tests", "build.rs"] {
            visit(
                &crate_root.join(target),
                &sanctioned,
                &openers,
                &mut offenders,
            );
        }

        assert!(
            offenders.is_empty(),
            "user data may only be removed by core/quarantine.rs; found: {offenders:?}"
        );
    }

    /// Security review C: after verification the source used to be removed by
    /// path, so a file that arrived in between was deleted without ever having
    /// been copied.
    #[test]
    fn a_file_added_after_verification_stays_at_the_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        write(&tree, "a.txt", b"first");
        write(&tree, "sub/b.txt", b"second");
        let target = dir.path().join("moved");

        let late = tree.join("late.txt");
        let _hook = seams::on_after_verify({
            let late = late.clone();
            move |_| fs::write(&late, b"arrived after verification").expect("late file")
        });

        if let Err(failure) = move_path_with(&tree, &target, seams::forced_cross_volume) {
            panic!(
                "the move must succeed: {}",
                failure.into_error(&tree, &target)
            );
        }

        assert_eq!(
            fs::read(&late).expect("the late file survives"),
            b"arrived after verification"
        );
        assert!(!tree.join("a.txt").exists(), "verified files leave");
        assert!(!tree.join("sub").exists(), "an emptied directory leaves");
        assert_eq!(fs::read(target.join("a.txt")).expect("read"), b"first");
        assert_eq!(fs::read(target.join("sub/b.txt")).expect("read"), b"second");
        assert!(!target.join("late.txt").exists(), "it was never copied");
    }

    /// The pin is what closes C: from before the copy until the delete, no
    /// other program can open a source file for writing.
    #[cfg(windows)]
    #[test]
    fn a_pinned_source_cannot_be_opened_for_writing_during_the_move() {
        const ERROR_SHARING_VIOLATION: i32 = 32;
        let dir = tempfile::tempdir().expect("tempdir");
        let source = write(dir.path(), "pinned.bin", b"the original bytes");
        let target = dir.path().join("moved.bin");

        let seen = std::rc::Rc::new(std::cell::Cell::new(None));
        let _hook = seams::on_after_verify({
            let seen = std::rc::Rc::clone(&seen);
            move |path| {
                let refused = fs::OpenOptions::new().write(true).open(path).err();
                seen.set(refused.and_then(|err| err.raw_os_error()));
            }
        });

        if let Err(failure) = move_path_with(&source, &target, seams::forced_cross_volume) {
            panic!(
                "the move must succeed: {}",
                failure.into_error(&source, &target)
            );
        }

        assert_eq!(
            seen.get(),
            Some(ERROR_SHARING_VIOLATION),
            "a writer must be locked out between verification and delete"
        );
        assert_eq!(fs::read(&target).expect("read"), b"the original bytes");
    }

    /// Security review D: a failed copy used to `remove_dir_all` the directory
    /// it had created. On restore that directory is in the user's folder, and
    /// whatever another program put there in the meantime went with it.
    #[test]
    fn a_failed_copy_leaves_foreign_files_in_a_directory_it_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        write(&tree, "a.txt", b"ours");
        write(&tree, "only.txt", b"ours too");
        let target = dir.path().join("moved");

        let _hook = seams::on_after_create_dir(|created| {
            fs::write(created.join("foreign.txt"), b"someone else's").expect("foreign");
            fs::write(created.join("only.txt"), b"in the way").expect("squatter");
        });

        match move_path_with(&tree, &target, seams::forced_cross_volume) {
            Err(MoveFailure::Occupied) => {}
            Ok(()) => panic!("the copy must not overwrite a file it did not create"),
            Err(MoveFailure::Failed(err) | MoveFailure::Encrypted(err)) => {
                panic!("expected Occupied, got: {err}")
            }
        }

        assert_eq!(
            fs::read(target.join("foreign.txt")).expect("the foreign file survives"),
            b"someone else's"
        );
        assert_eq!(
            fs::read(target.join("only.txt")).expect("the squatter survives"),
            b"in the way"
        );
        assert!(!target.join("a.txt").exists(), "what this call copied goes");
        assert_eq!(fs::read(tree.join("a.txt")).expect("source"), b"ours");
        assert_eq!(
            fs::read(tree.join("only.txt")).expect("source"),
            b"ours too"
        );
    }

    /// A file another program is writing is refused before anything is
    /// copied: the pin cannot be taken, and nothing exists at the destination.
    #[cfg(windows)]
    #[test]
    fn a_source_file_open_for_writing_refuses_the_move_before_copying() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        write(&tree, "a.txt", b"first");
        let busy = write(&tree, "b.txt", b"being written");
        let target = dir.path().join("moved");

        let _writer = fs::OpenOptions::new()
            .write(true)
            .open(&busy)
            .expect("hold a writer");

        match move_path_with(&tree, &target, seams::forced_cross_volume) {
            Err(MoveFailure::Failed(err) | MoveFailure::Encrypted(err)) => assert!(
                err.to_string().contains("open in another program"),
                "the message must say why: {err}"
            ),
            Ok(()) => panic!("a file someone is writing must not be moved"),
            Err(MoveFailure::Occupied) => panic!("nothing was in the way; the source was busy"),
        }

        assert_eq!(fs::read(tree.join("a.txt")).expect("source"), b"first");
        assert_eq!(fs::read(&busy).expect("source"), b"being written");
        assert!(!target.exists(), "nothing is created before the pin");
    }

    /// Security review C2: a delete that failed partway used to make the move
    /// an error, and `take` answered the error by deleting the ledger row,
    /// leaving the only full copy in the quarantine with nothing pointing at it.
    #[test]
    fn a_failed_delete_after_others_succeeded_still_completes_the_move() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        for name in ["a.txt", "b.txt", "c.txt"] {
            write(&tree, name, name.as_bytes());
        }
        let target = dir.path().join("moved");

        let _fail = seams::fail_delete_at(1);
        if let Err(failure) = move_path_with(&tree, &target, seams::forced_cross_volume) {
            panic!(
                "once part of the source is gone the move must succeed: {}",
                failure.into_error(&tree, &target)
            );
        }

        for name in ["a.txt", "b.txt", "c.txt"] {
            assert_eq!(fs::read(target.join(name)).expect("copy"), name.as_bytes());
        }
        assert!(!tree.join("a.txt").exists());
        assert!(!tree.join("c.txt").exists());
        assert_eq!(fs::read(tree.join("b.txt")).expect("leftover"), b"b.txt");
    }

    /// C2 at the level that matters: the ledger row outlives a partial delete,
    /// so the quarantined copy stays restorable and purgeable.
    #[test]
    fn a_partial_delete_during_take_keeps_the_ledger_row() {
        let h = harness(RetentionPolicy::default());
        let tree = h.work.join("tree");
        for name in ["a.txt", "b.txt", "c.txt"] {
            write(&tree, name, name.as_bytes());
        }

        let _cross = seams::force_cross_volume();
        let _fail = seams::fail_delete_at(1);
        let item = h
            .quarantine
            .take(&h.conn, h.quarantine.start_batch(), &tree)
            .expect("a move that removed part of the source is still a move");

        let found = h
            .quarantine
            .find_item(&h.conn, item.id)
            .expect("find")
            .expect("the ledger row survives");
        assert!(found.is_held());
        for name in ["a.txt", "b.txt", "c.txt"] {
            assert!(item.quarantine_path.join(name).exists(), "{name} is held");
        }
        assert_eq!(fs::read(tree.join("b.txt")).expect("leftover"), b"b.txt");
    }

    /// If not one source file could be deleted, the move did not happen: the
    /// copy is cleaned up and the source is whole.
    #[test]
    fn a_failed_first_delete_cleans_the_copy_and_keeps_the_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        write(&tree, "a.txt", b"a");
        write(&tree, "sub/b.txt", b"b");
        let target = dir.path().join("moved");

        let _fail = seams::fail_delete_at(0);
        match move_path_with(&tree, &target, seams::forced_cross_volume) {
            Err(MoveFailure::Failed(err) | MoveFailure::Encrypted(err)) => assert!(
                err.to_string().contains("could not remove the original"),
                "the message must say why: {err}"
            ),
            Ok(()) => panic!("nothing was removed, so nothing moved"),
            Err(MoveFailure::Occupied) => panic!("nothing was in the way"),
        }

        assert!(!target.exists(), "the copy is cleaned up");
        assert_eq!(fs::read(tree.join("a.txt")).expect("source"), b"a");
        assert_eq!(fs::read(tree.join("sub/b.txt")).expect("source"), b"b");
    }

    /// Same-volume rename moves read-only files, so the copy path must too,
    /// and the copy keeps the attribute.
    #[test]
    fn read_only_files_move_across_volumes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        let inner = write(&tree, "locked.txt", b"inside a directory");
        let single = write(dir.path(), "single.txt", b"on its own");
        for path in [&inner, &single] {
            let mut permissions = fs::metadata(path).expect("metadata").permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).expect("make read-only");
        }

        let moved_tree = dir.path().join("moved-tree");
        let moved_single = dir.path().join("moved-single.txt");
        for (from, to) in [(&tree, &moved_tree), (&single, &moved_single)] {
            if let Err(failure) = move_path_with(from, to, seams::forced_cross_volume) {
                panic!("a read-only file moves: {}", failure.into_error(from, to));
            }
        }

        assert!(!tree.exists(), "the source tree is gone");
        assert!(!single.exists(), "the source file is gone");
        for copy in [moved_tree.join("locked.txt"), moved_single] {
            let metadata = fs::metadata(&copy).expect("copy");
            assert!(metadata.permissions().readonly(), "{}", copy.display());
        }
    }

    /// `dirs_are_identical` used to drop listing entries it could not read,
    /// so verification could pass without comparing what it could not see.
    #[test]
    fn an_unreadable_directory_entry_fails_closed() {
        let name = std::ffi::OsString::from;
        let unreadable = vec![
            Ok(name("b")),
            Err(std::io::Error::other("unreadable entry")),
            Ok(name("a")),
        ];
        assert!(
            sorted_names(unreadable.into_iter()).is_err(),
            "an entry that cannot be read is not an entry that can be skipped"
        );

        let readable = vec![Ok(name("b")), Ok(name("a"))];
        assert_eq!(
            sorted_names(readable.into_iter()).expect("readable"),
            vec![name("a"), name("b")]
        );
    }

    /// The accounting behind C2: what a partial delete removed and what it
    /// left, including the directory that could not be emptied.
    #[test]
    fn a_partial_delete_reports_what_it_removed_and_what_it_left() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        for name in ["a.txt", "b.txt", "c.txt"] {
            write(&tree, name, name.as_bytes());
        }
        let target = dir.path().join("moved");

        let _fail = seams::fail_delete_at(1);
        let report = match move_across_volumes(&tree, &target) {
            Ok(report) => report,
            Err(failure) => panic!("{}", failure.into_error(&tree, &target)),
        };

        assert_eq!(report.deleted_files, 2);
        assert_eq!(report.leftovers, vec![tree.join("b.txt"), tree.clone()]);
        for name in ["a.txt", "b.txt", "c.txt"] {
            assert_eq!(fs::read(target.join(name)).expect("copy"), name.as_bytes());
        }
    }

    /// Security review D at the level of the cleanup itself: a file that is no
    /// longer the one this call created, a file whose identity was never
    /// learned, and a directory that is not empty all stay.
    #[test]
    fn cleanup_leaves_what_is_no_longer_the_one_it_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let swapped = write(dir.path(), "swapped.txt", b"someone else's file now");
        let unknown = write(dir.path(), "unknown.txt", b"identity never learned");
        let foreign = write(dir.path(), "sub/foreign.txt", b"arrived later");
        let sub = dir.path().join("sub");

        cleanup_created(&Created {
            files: vec![
                CreatedFile {
                    path: swapped.clone(),
                    id: Some(FileId {
                        volume: 0,
                        index: 0,
                    }),
                },
                CreatedFile {
                    path: unknown.clone(),
                    id: None,
                },
            ],
            dirs: vec![sub.clone()],
        });

        assert_eq!(
            fs::read(&swapped).expect("swapped stays"),
            b"someone else's file now"
        );
        assert_eq!(
            fs::read(&unknown).expect("unknown stays"),
            b"identity never learned"
        );
        assert_eq!(fs::read(&foreign).expect("foreign stays"), b"arrived later");
        assert!(sub.is_dir(), "a directory that is not empty stays");
    }

    /// `path:name`, the way Windows names an alternate data stream.
    #[cfg(windows)]
    fn stream_of(path: &Path, name: &str) -> PathBuf {
        let mut joined = path.as_os_str().to_os_string();
        joined.push(":");
        joined.push(name);
        PathBuf::from(joined)
    }

    /// Set exactly `attributes` on `path`, the way Explorer's properties
    /// dialog or `attrib` would.
    #[cfg(windows)]
    fn set_attributes(path: &Path, attributes: u32) {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
        };

        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let value = if attributes == 0 {
            FILE_ATTRIBUTE_NORMAL
        } else {
            FILE_FLAGS_AND_ATTRIBUTES(attributes)
        };
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        unsafe { SetFileAttributesW(PCWSTR(wide.as_ptr()), value) }.expect("set attributes");
    }

    /// Item 2 of the cross-volume review: the mark of the web is a named
    /// stream, and a copy of the main stream alone quietly dropped it, so a
    /// restored download no longer warned before it ran.
    #[cfg(windows)]
    #[test]
    fn a_zone_identifier_comes_across_with_the_file() {
        const ZONE: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        let inner = write(&tree, "download.exe", b"inside a directory");
        let single = write(dir.path(), "single.exe", b"on its own");
        for path in [&inner, &single] {
            fs::write(stream_of(path, "Zone.Identifier"), ZONE).expect("mark of the web");
        }

        let moved_tree = dir.path().join("moved-tree");
        let moved_single = dir.path().join("moved-single.exe");
        for (from, to) in [(&tree, &moved_tree), (&single, &moved_single)] {
            if let Err(failure) = move_path_with(from, to, seams::forced_cross_volume) {
                panic!("the move must succeed: {}", failure.into_error(from, to));
            }
        }

        assert!(!tree.exists(), "the source tree is gone");
        assert!(!single.exists(), "the source file is gone");
        for copy in [moved_tree.join("download.exe"), moved_single] {
            assert_eq!(
                fs::read(stream_of(&copy, "Zone.Identifier")).expect("the mark comes across"),
                ZONE,
                "{}",
                copy.display()
            );
        }
    }

    /// Any other named stream cannot be carried faithfully, so the whole move
    /// is refused before anything exists at the destination.
    #[cfg(windows)]
    #[test]
    fn a_file_with_any_other_named_stream_is_refused_before_copying() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        write(&tree, "a.txt", b"first");
        let carrier = write(&tree, "b.txt", b"second");
        fs::write(stream_of(&carrier, "notes"), b"hidden data").expect("stream");
        let target = dir.path().join("moved");

        match move_path_with(&tree, &target, seams::forced_cross_volume) {
            Err(MoveFailure::Failed(err) | MoveFailure::Encrypted(err)) => assert!(
                err.to_string().contains("alternate data stream"),
                "the message must say why: {err}"
            ),
            Ok(()) => panic!("a file with an extra stream must not be moved"),
            Err(MoveFailure::Occupied) => panic!("nothing was in the way"),
        }

        assert!(!target.exists(), "nothing is created before the refusal");
        assert_eq!(fs::read(tree.join("a.txt")).expect("source"), b"first");
        assert_eq!(fs::read(&carrier).expect("source"), b"second");
        assert_eq!(
            fs::read(stream_of(&carrier, "notes")).expect("the stream survives"),
            b"hidden data"
        );
    }

    /// NTFS shares per stream, so the pin on the main stream does not stop a
    /// new stream being written. One that appears after verification was never
    /// copied, so its file must stay at the source.
    #[cfg(windows)]
    #[test]
    fn a_stream_added_after_verification_keeps_its_file_at_the_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        write(&tree, "a.txt", b"first");
        let late_carrier = write(&tree, "b.txt", b"second");
        let target = dir.path().join("moved");

        let _hook = seams::on_after_verify({
            let late = stream_of(&late_carrier, "late");
            move |_| fs::write(&late, b"arrived after verification").expect("late stream")
        });

        if let Err(failure) = move_path_with(&tree, &target, seams::forced_cross_volume) {
            panic!(
                "the move must succeed: {}",
                failure.into_error(&tree, &target)
            );
        }

        assert!(!tree.join("a.txt").exists(), "an unchanged file leaves");
        assert_eq!(fs::read(&late_carrier).expect("it stays"), b"second");
        assert_eq!(
            fs::read(stream_of(&late_carrier, "late")).expect("the late stream survives"),
            b"arrived after verification"
        );
        assert_eq!(fs::read(target.join("b.txt")).expect("copy"), b"second");
    }

    /// The same race on a lone file: nothing has moved yet, so the copy is
    /// undone and the original stays whole.
    #[cfg(windows)]
    #[test]
    fn a_lone_file_whose_streams_change_after_verification_is_not_moved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let single = write(dir.path(), "single.txt", b"on its own");
        let target = dir.path().join("moved.txt");

        let _hook = seams::on_after_verify({
            let late = stream_of(&single, "late");
            move |_| fs::write(&late, b"arrived after verification").expect("late stream")
        });

        assert!(
            matches!(
                move_path_with(&single, &target, seams::forced_cross_volume),
                Err(MoveFailure::Failed(_))
            ),
            "a file that changed after verification must not be moved"
        );
        assert_eq!(fs::read(&single).expect("source"), b"on its own");
        assert_eq!(
            fs::read(stream_of(&single, "late")).expect("the late stream survives"),
            b"arrived after verification"
        );
        assert!(!target.exists(), "the copy is undone");
    }

    /// Item 4: hidden, system, archive, read-only and not-indexed come across
    /// for files and directories alike, including a cleared archive flag.
    #[cfg(windows)]
    #[test]
    fn file_and_directory_attributes_come_across() {
        use std::os::windows::fs::MetadataExt;
        const READONLY: u32 = 0x1;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        const ARCHIVE: u32 = 0x20;
        const NOT_INDEXED: u32 = 0x2000;
        const KEPT: u32 = READONLY | HIDDEN | SYSTEM | ARCHIVE | NOT_INDEXED;

        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        let expected = [
            ("sub/plain.txt", 0),
            ("flagged.txt", HIDDEN | SYSTEM | READONLY | NOT_INDEXED),
            ("sub", HIDDEN | SYSTEM | READONLY),
            ("", HIDDEN | ARCHIVE),
        ];
        write(&tree, "sub/plain.txt", b"no flags at all");
        write(&tree, "flagged.txt", b"hidden and system");
        for (relative, attributes) in expected {
            set_attributes(&join_relative(&tree, Path::new(relative)), attributes);
        }

        let target = dir.path().join("moved");
        if let Err(failure) = move_path_with(&tree, &target, seams::forced_cross_volume) {
            panic!(
                "the move must succeed: {}",
                failure.into_error(&tree, &target)
            );
        }

        for (relative, attributes) in expected {
            let copy = join_relative(&target, Path::new(relative));
            let got = fs::symlink_metadata(&copy).expect("copy").file_attributes() & KEPT;
            assert_eq!(got, attributes, "`{relative}`: {got:#x} != {attributes:#x}");
        }
        assert!(
            !tree.exists(),
            "the source leaves, read-only directory included"
        );
    }

    /// Item 3: a top-level directory swapped for a junction after it was
    /// inspected used to be walked through, so the junction's target was
    /// copied and then emptied.
    #[cfg(windows)]
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn a_source_directory_swapped_for_a_junction_is_never_walked_through() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        let real = write(&tree, "real.txt", b"part of the move");
        let elsewhere = dir.path().join("elsewhere");
        let victim = write(&elsewhere, "victim.txt", b"not part of the move");
        let aside = dir.path().join("aside");
        let target = dir.path().join("moved");

        let _hook = seams::on_before_walk({
            let (tree, aside, elsewhere) = (tree.clone(), aside.clone(), elsewhere.clone());
            move |_| {
                fs::rename(&tree, &aside).expect("move the real tree aside");
                assert!(
                    crate::core::test_support::make_junction(&tree, &elsewhere),
                    "`mklink /J` needs no privilege"
                );
            }
        });

        match move_path_with(&tree, &target, seams::forced_cross_volume) {
            Err(MoveFailure::Failed(err) | MoveFailure::Encrypted(err)) => assert!(
                err.to_string().contains("link"),
                "the refusal must say why: {err}"
            ),
            Ok(()) => panic!("a junction must never be walked through"),
            Err(MoveFailure::Occupied) => panic!("nothing was in the way"),
        }

        assert_eq!(
            fs::read(&victim).expect("the junction's target is untouched"),
            b"not part of the move"
        );
        assert!(!target.exists(), "nothing is created");
        assert!(tree.exists(), "the junction itself stays");
        assert_eq!(
            fs::read(aside.join("real.txt")).expect("the real tree is untouched"),
            b"part of the move"
        );
        drop(real);
    }

    /// The final-path check behind item 3 compares whole components and
    /// ignores case, as NTFS does.
    #[cfg(windows)]
    #[test]
    fn containment_is_by_whole_component_and_ignores_case() {
        let root = Path::new(r"\\?\C:\Tree");
        assert!(is_within_root(root, Path::new(r"\\?\c:\tree\Sub\a.txt")));
        assert!(!is_within_root(root, Path::new(r"\\?\C:\Treehouse\a.txt")));
        assert!(!is_within_root(root, Path::new(r"\\?\D:\Tree\a.txt")));
    }

    /// Item 6: `MoveFileExW` without the `\\?\` prefix stops at MAX_PATH and
    /// answered a long quarantine path with "path not found".
    #[cfg(windows)]
    #[test]
    fn a_rename_handles_paths_longer_than_max_path() {
        const MAX_PATH: usize = 260;
        let dir = tempfile::tempdir().expect("tempdir");
        let deep = ["a", "b", "c"]
            .iter()
            .fold(dir.path().to_path_buf(), |path, part| {
                path.join(part.repeat(100))
            });
        let from = write(&deep, "from.txt", b"far down");
        let to = deep.join("to.txt");
        assert!(to.as_os_str().len() > MAX_PATH);

        rename_no_replace(&from, &to).expect("a long path moves");
        assert_eq!(fs::read(&to).expect("moved"), b"far down");

        let occupied = write(&deep, "from.txt", b"in the way");
        let err = rename_no_replace(&to, &occupied).expect_err("still never replaces");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&occupied).expect("kept"), b"in the way");
    }

    /// Item 4 of the cross-volume review: directory attributes now come
    /// across, read-only among them, so undoing a failed move must still be
    /// able to remove a read-only directory it created.
    #[cfg(windows)]
    #[test]
    fn a_failed_move_cleans_up_read_only_directories_it_created() {
        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY;

        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        write(&tree, "a.txt", b"a");
        write(&tree, "sub/b.txt", b"b");
        set_attributes(&tree.join("sub"), FILE_ATTRIBUTE_READONLY.0);
        let target = dir.path().join("moved");

        let _fail = seams::fail_delete_at(0);
        let outcome = move_path_with(&tree, &target, seams::forced_cross_volume);
        let left_behind = target.exists();
        set_attributes(&tree.join("sub"), 0);

        assert!(
            matches!(outcome, Err(MoveFailure::Failed(_))),
            "nothing was removed, so nothing moved"
        );
        assert!(
            !left_behind,
            "the copy, read-only directory included, is cleaned up"
        );
        assert_eq!(fs::read(tree.join("sub/b.txt")).expect("source"), b"b");
    }

    /// Every file under `root`, at any depth.
    fn files_under(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = fs::read_dir(root) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files_under(&path));
            } else {
                found.push(path);
            }
        }
        found
    }

    /// Adversarial review, LOW 1: a take-back that reports success while the
    /// delete stays pending used to be believed. The copy was then cleaned up
    /// as if the source were whole, and the source went when its pin closed:
    /// both gone. An unconfirmed take-back must count as a delete, so the
    /// verified copy stays.
    #[cfg(windows)]
    #[test]
    fn a_take_back_that_does_not_hold_keeps_the_verified_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let single = write(dir.path(), "single.txt", b"on its own");
        let target = dir.path().join("moved.txt");

        let _hook = seams::on_after_verify({
            let late = stream_of(&single, "late");
            move |_| fs::write(&late, b"arrived after verification").expect("late stream")
        });
        let _skip = seams::skip_revoke();
        let outcome = move_path_with(&single, &target, seams::forced_cross_volume);

        assert_eq!(
            fs::read(&target).expect("the verified copy must survive"),
            b"on its own"
        );
        assert!(!single.exists(), "the delete could not be taken back");
        assert!(
            outcome.is_ok(),
            "the source is gone and the copy is whole, so the move happened"
        );
    }

    /// The same race through the real take-back and the ledger: the stream
    /// recheck marks the delete, takes it back for real, and the take fails
    /// cleanly: source and its late stream whole, no copy in the quarantine,
    /// no ledger row.
    #[cfg(windows)]
    #[test]
    fn a_late_stream_during_take_is_taken_back_for_real() {
        let h = harness(RetentionPolicy::default());
        let single = write(&h.work, "single.txt", b"on its own");

        let _cross = seams::force_cross_volume();
        let _hook = seams::on_after_verify({
            let late = stream_of(&single, "late");
            move |_| fs::write(&late, b"arrived after verification").expect("late stream")
        });
        let refused = h
            .quarantine
            .take(&h.conn, h.quarantine.start_batch(), &single);

        assert!(refused.is_err(), "a file that changed must not be taken");
        assert_eq!(fs::read(&single).expect("source"), b"on its own");
        assert_eq!(
            fs::read(stream_of(&single, "late")).expect("the late stream survives"),
            b"arrived after verification"
        );
        let held = files_under(&h._dir.path().join("quarantine"));
        assert!(held.is_empty(), "no copy may stay behind: {held:?}");
        assert!(
            h.quarantine.list_items(&h.conn).expect("list").is_empty(),
            "no ledger row for a take that did not happen"
        );
    }

    /// The primitive the take-back check rests on: a marked delete reads as
    /// pending through the same handle, and a real take-back clears it.
    #[cfg(windows)]
    #[test]
    fn a_marked_delete_reads_as_pending_until_taken_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), "a.txt", b"payload");
        let handle = pinned::open_pinned(&path).expect("pin");

        assert!(!pinned::standard_info(&handle).expect("info").delete_pending);
        delete_by_handle(&handle).expect("mark");
        assert!(pinned::standard_info(&handle).expect("info").delete_pending);
        undelete_by_handle(&handle).expect("take back");
        assert!(!pinned::standard_info(&handle).expect("info").delete_pending);
        drop(handle);
        assert_eq!(fs::read(&path).expect("still there"), b"payload");
    }

    /// Encrypt `path` with Windows EFS, or say why this machine cannot.
    #[cfg(windows)]
    fn encrypt(path: &Path) -> std::result::Result<(), String> {
        use std::os::windows::fs::MetadataExt;
        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_ENCRYPTED;

        let output = std::process::Command::new("cipher")
            .arg("/e")
            .arg("/a")
            .arg(path)
            .output()
            .map_err(|err| format!("cannot run cipher: {err}"))?;
        let attributes = fs::metadata(path)
            .map_err(|err| format!("cannot inspect {}: {err}", path.display()))?
            .file_attributes();
        if attributes & FILE_ATTRIBUTE_ENCRYPTED.0 == 0 {
            return Err(format!(
                "cipher /e did not encrypt {} ({}): {}",
                path.display(),
                output.status,
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
        Ok(())
    }

    /// Adversarial review, LOW 2: a copy to another drive is written in the
    /// clear, so an EFS-encrypted file or directory must not cross. A rename
    /// on one volume keeps the encryption and still works.
    ///
    /// Needs a volume with EFS (NTFS on Windows Pro or above). The first run
    /// on a machine makes Windows create the user's own EFS certificate.
    /// `cipher` is only ever run on paths inside this test's temporary
    /// directory.
    #[cfg(windows)]
    #[test]
    fn encrypted_files_are_refused_across_volumes_but_renamed_on_one() {
        const TEST: &str = "encrypted_files_are_refused_across_volumes_but_renamed_on_one";
        if !crate::core::test_support::efs_tests_enabled(TEST) {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let single = write(dir.path(), "single.txt", b"secret");
        let tree = dir.path().join("tree");
        fs::create_dir_all(&tree).expect("tree");
        if let Err(detail) = encrypt(&single).and_then(|()| encrypt(&tree)) {
            crate::core::test_support::privileged_operation_unavailable(TEST, &detail);
            return;
        }
        // Created inside an encrypted directory, so encrypted too.
        write(&tree, "inner.txt", b"also secret");

        for (source, target) in [
            (&single, dir.path().join("moved.txt")),
            (&tree, dir.path().join("moved")),
        ] {
            match move_path_with(source, &target, seams::forced_cross_volume) {
                // Typed, not recognised by its wording (round-2 review).
                Err(MoveFailure::Encrypted(err)) => assert!(
                    err.to_string().contains("encrypted with Windows EFS"),
                    "the message must say why: {err}"
                ),
                Ok(()) => panic!("{} crossed volumes in the clear", source.display()),
                Err(MoveFailure::Failed(err)) => panic!("an untyped refusal: {err}"),
                Err(MoveFailure::Occupied) => panic!("nothing was in the way"),
            }
            assert!(
                !target.exists(),
                "nothing is created for {}",
                target.display()
            );
        }
        assert_eq!(fs::read(&single).expect("source"), b"secret");
        assert_eq!(
            fs::read(tree.join("inner.txt")).expect("source"),
            b"also secret"
        );

        let renamed = dir.path().join("renamed.txt");
        if let Err(failure) = move_path_with(&single, &renamed, rename_no_replace) {
            panic!(
                "a rename on one volume must still work: {}",
                failure.into_error(&single, &renamed)
            );
        }
        assert_eq!(fs::read(&renamed).expect("renamed"), b"secret");
        // Everything `cipher` touched is inside this temporary directory;
        // removing it is checked rather than left to a silent drop.
        dir.close().expect("the encrypted test files are removed");
    }

    /// Adversarial review, LOW 3: a file with two names used to fail at its
    /// second pin with a misleading "open in another program". It is refused
    /// for what it is, before anything is created, inside a tree or alone.
    #[test]
    fn hard_linked_files_are_refused_across_volumes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        let first = write(&tree, "a.txt", b"one file");
        fs::hard_link(&first, tree.join("b.txt")).expect("hard link inside the tree");
        let single = write(dir.path(), "single.txt", b"two names");
        fs::hard_link(&single, dir.path().join("elsewhere.txt")).expect("hard link");

        for (source, target) in [
            (&tree, dir.path().join("moved")),
            (&single, dir.path().join("moved.txt")),
        ] {
            match move_path_with(source, &target, seams::forced_cross_volume) {
                Err(MoveFailure::Failed(err) | MoveFailure::Encrypted(err)) => assert!(
                    err.to_string().contains("more than one name (hard link)"),
                    "the message must say why: {err}"
                ),
                Ok(()) => panic!("{} was moved", source.display()),
                Err(MoveFailure::Occupied) => panic!("nothing was in the way"),
            }
            assert!(
                !target.exists(),
                "nothing is created for {}",
                target.display()
            );
        }
        assert_eq!(fs::read(tree.join("a.txt")).expect("source"), b"one file");
        assert_eq!(fs::read(tree.join("b.txt")).expect("source"), b"one file");
        assert_eq!(fs::read(&single).expect("source"), b"two names");
    }

    /* ---------------------------------------------------------------------
     * Adversarial review follow-up
     * ------------------------------------------------------------------ */

    fn warning(path: &Path, reason: &str) -> MoveWarning {
        MoveWarning {
            path: path.display().to_string(),
            reason: reason.to_owned(),
        }
    }

    /// Follow-up A: what a partial take left behind reaches its result, not
    /// only the log.
    #[test]
    fn a_partial_take_reports_what_stayed_behind() {
        let h = harness(RetentionPolicy::default());
        let tree = h.work.join("tree");
        for name in ["a.txt", "b.txt", "c.txt"] {
            write(&tree, name, name.as_bytes());
        }

        let _cross = seams::force_cross_volume();
        let _fail = seams::fail_delete_at(1);
        let taken = h
            .quarantine
            .take_reporting(&h.conn, h.quarantine.start_batch(), &tree)
            .expect("something left the source, so the take happened");

        assert_eq!(
            taken.warnings,
            vec![
                warning(&tree.join("b.txt"), LEFT_FILE),
                warning(&tree, LEFT_DIR)
            ]
        );
        assert!(tree.join("b.txt").exists(), "the refused file stays");
        // Round-2 review: what stayed is not reclaimed.
        assert_eq!(taken.item.size_bytes, 15);
        assert_eq!(taken.left_behind_bytes, "b.txt".len() as u64);
    }

    /// Follow-up A, the other direction: a partial restore says what stayed
    /// in the quarantine.
    #[test]
    fn a_partial_restore_reports_what_stayed_behind() {
        let h = harness(RetentionPolicy::default());
        let tree = h.work.join("tree");
        for name in ["a.txt", "b.txt", "c.txt"] {
            write(&tree, name, name.as_bytes());
        }
        let item = h
            .quarantine
            .take(&h.conn, h.quarantine.start_batch(), &tree)
            .expect("take on one volume");

        let _cross = seams::force_cross_volume();
        let _fail = seams::fail_delete_at(1);
        let outcome = h
            .quarantine
            .restore_item(&h.conn, item.id)
            .expect("something left the quarantine, so the restore happened");

        assert_eq!(outcome.restored_to, tree);
        assert_eq!(
            outcome.warnings,
            vec![
                warning(&item.quarantine_path.join("b.txt"), LEFT_FILE),
                warning(&item.quarantine_path, LEFT_DIR)
            ]
        );
    }

    /// Follow-up B: after a refused delete, only "not found" or another file
    /// at the name means the source went. An identity that cannot be read
    /// keeps both, and the message says where each one is.
    #[test]
    fn an_unreadable_identity_after_a_refused_delete_keeps_both() {
        let dir = tempfile::tempdir().expect("tempdir");
        let single = write(dir.path(), "single.txt", b"payload");
        let target = dir.path().join("moved.txt");

        let _fail = seams::fail_delete_at(0);
        let _unreadable = seams::fail_identity_read();
        let report = match move_across_volumes(&single, &target) {
            Ok(report) => report,
            Err(failure) => panic!(
                "an unknown state must keep the copy and its record, not fail: {}",
                failure.into_error(&single, &target)
            ),
        };

        assert_eq!(fs::read(&single).expect("the original stays"), b"payload");
        assert_eq!(fs::read(&target).expect("the copy stays too"), b"payload");
        assert_eq!(report.deleted_files, 0, "nothing is known to have gone");
        assert_eq!(report.warnings, vec![warning(&single, BOTH_KEPT)]);
    }

    /// Round-2 review, HIGH: a take whose source cannot be told after its
    /// refused delete keeps its ledger row, or the copy in the quarantine
    /// would be orphaned.
    #[test]
    fn an_unknown_source_after_a_refused_delete_keeps_the_take_row() {
        let h = harness(RetentionPolicy::default());
        let single = write(&h.work, "single.txt", b"payload");

        let _cross = seams::force_cross_volume();
        let _fail = seams::fail_delete_at(0);
        let _unreadable = seams::fail_identity_read();
        let taken = h
            .quarantine
            .take_reporting(&h.conn, h.quarantine.start_batch(), &single)
            .expect("the copy exists and may be the only one, so the take stands");

        assert_eq!(taken.warnings, vec![warning(&single, BOTH_KEPT)]);
        assert_eq!(taken.left_behind_bytes, b"payload".len() as u64);
        assert_eq!(fs::read(&single).expect("the original stays"), b"payload");
        assert_eq!(
            fs::read(&taken.item.quarantine_path).expect("the copy stays"),
            b"payload"
        );
        let row = h
            .quarantine
            .find_item(&h.conn, taken.item.id)
            .expect("query")
            .expect("the ledger row survives");
        assert!(row.is_held());
    }

    /// Follow-up C, the first revoke fix on its own: a take-back that reports
    /// success while the delete stays pending is an error.
    #[cfg(windows)]
    #[test]
    fn a_take_back_reported_done_but_still_pending_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let single = write(dir.path(), "single.txt", b"kept");
        let handle = pinned::open_to_delete(&single).expect("pin");
        delete_by_handle(&handle).expect("mark the delete");

        {
            let _skip = seams::skip_revoke();
            assert!(
                undelete_by_handle(&handle).is_err(),
                "a take-back that did nothing must not read as done"
            );
        }
        undelete_by_handle(&handle).expect("the real take-back");
        drop(handle);
        assert_eq!(fs::read(&single).expect("the file stays"), b"kept");
    }

    /// Follow-up C, the second revoke fix on its own: a first delete that
    /// reports failure but took effect leaves only the copy, which stays.
    #[cfg(windows)]
    #[test]
    fn a_refused_delete_that_took_effect_keeps_the_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let single = write(dir.path(), "single.txt", b"on its own");
        let target = dir.path().join("moved.txt");

        let _gone = seams::fail_delete_after_marking_at(0);
        let report = match move_across_volumes(&single, &target) {
            Ok(report) => report,
            Err(failure) => panic!("{}", failure.into_error(&single, &target)),
        };

        assert!(!single.exists(), "the delete took effect");
        assert_eq!(
            fs::read(&target).expect("the only copy must stay"),
            b"on its own"
        );
        assert_eq!(report.deleted_files, 1);
        assert_eq!(report.warnings, vec![warning(&single, COPY_ONLY)]);
    }

    /// Follow-up D: a late stream whose take-back fails is lost with its
    /// file. That file is not counted as moved, and the user is told.
    #[cfg(windows)]
    #[test]
    fn a_late_stream_that_could_not_be_kept_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let single = write(dir.path(), "single.txt", b"on its own");
        let target = dir.path().join("moved.txt");

        let _hook = seams::on_after_verify({
            let late = stream_of(&single, "late");
            move |_| fs::write(&late, b"arrived after verification").expect("late stream")
        });
        let _skip = seams::skip_revoke();
        let report = match move_across_volumes(&single, &target) {
            Ok(report) => report,
            Err(failure) => panic!("{}", failure.into_error(&single, &target)),
        };

        assert_eq!(fs::read(&target).expect("the copy stays"), b"on its own");
        assert!(!single.exists(), "the take-back did not hold");
        assert_eq!(report.deleted_files, 0, "its delete did not go as asked");
        assert_eq!(report.warnings, vec![warning(&single, STREAM_LOST)]);
    }

    /// Round-2 review: a take-back that went through but could not be
    /// confirmed is not a loss. It has its own reason, and the copy stays.
    #[cfg(windows)]
    #[test]
    fn an_unconfirmed_take_back_is_not_reported_as_a_loss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let single = write(dir.path(), "single.txt", b"on its own");
        let target = dir.path().join("moved.txt");

        let _hook = seams::on_after_verify({
            let late = stream_of(&single, "late");
            move |_| fs::write(&late, b"arrived after verification").expect("late stream")
        });
        let _unconfirmed = seams::fail_revoke_confirmation();
        let report = match move_across_volumes(&single, &target) {
            Ok(report) => report,
            Err(failure) => panic!("{}", failure.into_error(&single, &target)),
        };

        assert_eq!(fs::read(&target).expect("the copy stays"), b"on its own");
        assert!(single.exists(), "the take-back itself went through");
        assert_eq!(report.deleted_files, 0);
        assert_eq!(report.warnings, vec![warning(&single, STREAM_UNCONFIRMED)]);
        assert_eq!(report.left_bytes, b"on its own".len() as u64);
    }

    /// Follow-up F: an EFS-encrypted item cannot be restored across drives.
    /// The refusal says what to do, and the item stays held.
    ///
    /// `cipher` is only run on a path inside this test's temporary directory.
    #[cfg(windows)]
    #[test]
    fn an_encrypted_item_refused_across_drives_stays_held() {
        const TEST: &str = "an_encrypted_item_refused_across_drives_stays_held";
        if !crate::core::test_support::efs_tests_enabled(TEST) {
            return;
        }
        let h = harness(RetentionPolicy::default());
        let single = write(&h.work, "single.txt", b"secret");
        let item = h
            .quarantine
            .take(&h.conn, h.quarantine.start_batch(), &single)
            .expect("take on one volume");
        if let Err(detail) = encrypt(&item.quarantine_path) {
            crate::core::test_support::privileged_operation_unavailable(TEST, &detail);
            return;
        }

        let _cross = seams::force_cross_volume();
        let err = h
            .quarantine
            .restore_item(&h.conn, item.id)
            .expect_err("an encrypted item must not cross in the clear");

        let message = err.to_string();
        assert!(message.contains(EFS_REFUSAL), "{message}");
        assert!(message.contains("stays in the quarantine"), "{message}");
        assert!(message.contains("until you remove it"), "{message}");
        assert!(message.contains("decrypt"), "{message}");
        let held = h
            .quarantine
            .find_item(&h.conn, item.id)
            .expect("ledger")
            .expect("the row stays");
        assert!(held.is_held(), "a refused restore leaves the item held");
        assert!(
            held.restore_refused_at.is_some(),
            "the refusal is recorded so the automatic purge keeps the item"
        );
        assert_eq!(fs::read(&item.quarantine_path).expect("held"), b"secret");
        assert!(!single.exists(), "nothing was written in the clear");
        drop(h.conn);
        h._dir
            .close()
            .expect("the encrypted test files are removed");
    }

    /// Round 4: when the refusal cannot be recorded, `restore_item` must not
    /// promise to keep the item. The message says the purge may still remove
    /// it, and the item stays held, unmarked.
    ///
    /// `cipher` is only run on a path inside this test's temporary directory.
    #[cfg(windows)]
    #[test]
    fn an_encrypted_refusal_that_could_not_be_marked_says_so() {
        const TEST: &str = "an_encrypted_refusal_that_could_not_be_marked_says_so";
        if !crate::core::test_support::efs_tests_enabled(TEST) {
            return;
        }
        let h = harness(RetentionPolicy::default());
        let single = write(&h.work, "single.txt", b"secret");
        let item = h
            .quarantine
            .take(&h.conn, h.quarantine.start_batch(), &single)
            .expect("take on one volume");
        if let Err(detail) = encrypt(&item.quarantine_path) {
            crate::core::test_support::privileged_operation_unavailable(TEST, &detail);
            return;
        }
        h.conn
            .execute_batch(
                "CREATE TRIGGER refuse_marker BEFORE UPDATE OF restore_refused_at ON quarantine
                 BEGIN SELECT RAISE(ABORT, 'marker refused'); END;",
            )
            .expect("trigger");

        let _cross = seams::force_cross_volume();
        let err = h
            .quarantine
            .restore_item(&h.conn, item.id)
            .expect_err("an encrypted item must not cross in the clear");

        let message = err.to_string();
        assert!(message.contains(EFS_REFUSAL), "{message}");
        assert!(message.contains("may still remove it"), "{message}");
        assert!(!message.contains("until you remove it"), "{message}");
        let held = h
            .quarantine
            .find_item(&h.conn, item.id)
            .expect("ledger")
            .expect("the row stays");
        assert!(held.is_held(), "a refused restore leaves the item held");
        assert!(held.restore_refused_at.is_none(), "the mark was refused");
        assert_eq!(fs::read(&item.quarantine_path).expect("held"), b"secret");
        assert!(!single.exists(), "nothing was written in the clear");
        drop(h.conn);
        h._dir
            .close()
            .expect("the encrypted test files are removed");
    }
}
