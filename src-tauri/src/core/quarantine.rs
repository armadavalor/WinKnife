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
//! Move semantics: `fs::rename` when source and destination share a volume
//! (atomic, instant), otherwise copy, verify byte-for-byte, and only then
//! remove the source. A failed verification leaves the original untouched.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};

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
        if !destination.starts_with(&self.root) {
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
        };

        // Ledger first: if the move succeeds and the process then dies, the row
        // already knows where the file came from. A row without a file is
        // recoverable; a file without a row is not.
        insert_item(conn, &item)?;

        if let Err(failure) = move_path(path, &destination) {
            // The move failed, so the row describes something that never
            // happened. Drop it rather than leaving a phantom in the drawer —
            // and say so out loud if even that does not work, because a
            // swallowed failure here is exactly the phantom it was meant to
            // prevent (security review M3).
            if let Err(cleanup) = conn.execute(
                "DELETE FROM quarantine WHERE id = ?1",
                [item_id.to_string()],
            ) {
                tracing::error!(
                    item = %item_id,
                    error = %cleanup,
                    "could not drop the ledger row for a move that never happened; \
                     the drawer may offer to restore an item that was never taken"
                );
            }
            return Err(failure.into_error(path, &destination));
        }

        tracing::info!(
            item = %item_id,
            batch = %batch,
            original = %path.display(),
            size_bytes,
            "quarantined"
        );
        Ok(item)
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

        // Mark the row before the move, mirroring `take`. A row that says
        // "restored" while the file is still here leaks a little disk; a row
        // that says "held" while the file has already gone home is a phantom
        // the drawer offers to restore a second time (security review M2).
        let now = Utc::now();
        conn.execute(
            "UPDATE quarantine SET restored_at = ?2 WHERE id = ?1",
            rusqlite::params![item_id.to_string(), timestamp(now)],
        )?;

        match self.move_out(&item) {
            Ok((target, renamed)) => {
                tracing::info!(
                    item = %item_id,
                    restored_to = %target.display(),
                    renamed,
                    "restored from quarantine"
                );
                Ok(RestoreOutcome {
                    item_id,
                    restored_to: target,
                    renamed,
                })
            }
            Err(err) => {
                if let Err(reset) = conn.execute(
                    "UPDATE quarantine SET restored_at = NULL WHERE id = ?1",
                    [item_id.to_string()],
                ) {
                    tracing::error!(
                        item = %item_id,
                        error = %reset,
                        "could not un-mark a restore that failed; the item is still held \
                         but the drawer will no longer show it"
                    );
                }
                Err(err)
            }
        }
    }

    /// Move one item back out, claiming its destination exclusively.
    ///
    /// The claim is the point. Probing with `Path::exists` and then renaming
    /// left a window in which something could appear at the target, and
    /// `fs::rename` maps onto `MoveFileEx` *with* `MOVEFILE_REPLACE_EXISTING`,
    /// so it would then be overwritten. Here the rename itself refuses an
    /// occupied destination and the loop simply tries the next name
    /// (security review H2).
    fn move_out(&self, item: &QuarantineItem) -> Result<(PathBuf, bool)> {
        let mut contested = 0u32;

        for attempt in 0..=RESTORE_ATTEMPTS {
            let (candidate, renamed) = restore_candidate(&item.original_path, attempt);
            if is_occupied(&candidate) {
                continue;
            }
            if let Some(parent) = candidate.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    CoreError::Quarantine(format!(
                        "cannot recreate `{}` for the restore: {err}",
                        parent.display()
                    ))
                })?;
            }

            match move_path(&item.quarantine_path, &candidate) {
                Ok(()) => return Ok((candidate, renamed)),
                // Something claimed the name between the probe and the move.
                Err(MoveFailure::Occupied) => {
                    contested += 1;
                    continue;
                }
                Err(failure) => return Err(failure.into_error(&item.quarantine_path, &candidate)),
            }
        }

        Err(CoreError::Quarantine(format!(
            "could not find a free name to restore `{}` to after {RESTORE_ATTEMPTS} attempts \
             ({contested} were taken while we were looking)",
            item.original_path.display()
        )))
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

    /// Remove items whose `purge_after` has passed.
    ///
    /// # Errors
    /// Returns an error only if the ledger cannot be read.
    pub fn purge_expired(&self, conn: &Connection, now: DateTime<Utc>) -> Result<PurgeReport> {
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at
             FROM quarantine
             WHERE restored_at IS NULL AND purge_after <= ?1
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

    /// Purge oldest-first until the total held size is back under the cap.
    ///
    /// # Errors
    /// Returns an error only if the ledger cannot be read.
    pub fn enforce_size_cap(&self, conn: &Connection) -> Result<PurgeReport> {
        let held = self.stats(conn)?;
        if held.total_bytes <= self.policy.max_bytes {
            return Ok(PurgeReport::default());
        }

        let mut over_by = held.total_bytes - self.policy.max_bytes;
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at
             FROM quarantine
             WHERE restored_at IS NULL
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

        tracing::warn!(
            held_bytes = held.total_bytes,
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
                    quarantined_at, purge_after, restored_at
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
                    quarantined_at, purge_after, restored_at
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
                    quarantined_at, purge_after, restored_at
             FROM quarantine
             WHERE restored_at IS NULL",
        )?;
        let (items, unreadable_rows) = collect_items(stmt.query_map([], row_to_item)?)?;
        Ok(QuarantineStats {
            item_count: items.len() as u64,
            total_bytes: items.iter().map(|item| item.size_bytes).sum(),
            unreadable_rows,
        })
    }

    /// Look one item up.
    ///
    /// # Errors
    /// Returns an error if the ledger cannot be read.
    pub fn find_item(&self, conn: &Connection, item_id: Uuid) -> Result<Option<QuarantineItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, original_path, quarantine_path, size_bytes,
                    quarantined_at, purge_after, restored_at
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
            if !item.quarantine_path.starts_with(&self.root) {
                tracing::error!(
                    item = %item.id,
                    path = %item.quarantine_path.display(),
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

    let (Some(id), Some(batch_id), Some(quarantined_at), Some(purge_after)) = (
        parse_uuid(&id),
        parse_uuid(&batch_id),
        parse_time(&quarantined_at),
        parse_time(&purge_after),
    ) else {
        return Ok(None);
    };
    let restored_at = match restored_at.as_deref() {
        Some(raw) => match parse_time(raw) {
            Some(value) => Some(value),
            None => return Ok(None),
        },
        None => None,
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
    }))
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

/// Why a move did not happen.
enum MoveFailure {
    /// Something already occupies the destination. The caller picks another
    /// name; it never overwrites.
    Occupied,
    /// Anything else, already worded for the user.
    Failed(CoreError),
}

impl MoveFailure {
    fn into_error(self, from: &Path, to: &Path) -> CoreError {
        match self {
            Self::Occupied => CoreError::Quarantine(format!(
                "cannot move `{}` to `{}`: something is already there",
                from.display(),
                to.display()
            )),
            Self::Failed(err) => err,
        }
    }
}

/// The rename primitive. A function pointer so a test can force the
/// cross-volume branch without a second physical volume (security review M6).
type Rename = fn(&Path, &Path) -> std::io::Result<()>;

/// Rename when we can, copy-verify-remove when we must. Never replaces.
fn move_path(from: &Path, to: &Path) -> std::result::Result<(), MoveFailure> {
    move_path_with(from, to, rename_no_replace)
}

fn move_path_with(from: &Path, to: &Path, rename: Rename) -> std::result::Result<(), MoveFailure> {
    match rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(MoveFailure::Occupied),
        Err(err) if is_cross_volume(&err) => {
            copy_verify_remove(from, to).map_err(MoveFailure::Failed)
        }
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
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let wide = |path: &Path| -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let (from_wide, to_wide) = (wide(from), wide(to));

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

/// Cross-volume move: copy, prove the copy is identical, then remove the source.
///
/// The order matters. If verification fails the original is still there and the
/// half-written copy is cleaned up, so a bad move costs the user nothing.
#[allow(clippy::disallowed_methods)]
fn copy_verify_remove(from: &Path, to: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(from).map_err(|err| {
        CoreError::Quarantine(format!("cannot inspect `{}`: {err}", from.display()))
    })?;

    // What the cleanup below may touch is decided by whether *this call* made
    // the destination, and only the copy itself knows that. Probing `to` first
    // answers a different question: a file that appears between the probe and
    // `create_new` makes the copy fail with `AlreadyExists` while the probe
    // still says "did not exist", and on the restore path `to` is a user path —
    // so the cleanup deleted a file this call never created (security review
    // H1, then M10).
    let mut created_destination = false;

    let copied = if metadata.is_dir() {
        copy_dir_recursive(from, to, &mut created_destination)
    } else {
        copy_file(from, to, &mut created_destination).map(|()| Vec::new())
    };
    let skipped_links = match copied {
        Ok(skipped) => skipped,
        Err(err) => {
            cleanup_partial(to, created_destination);
            return Err(CoreError::Quarantine(format!(
                "cannot copy `{}` to `{}`: {err}",
                from.display(),
                to.display()
            )));
        }
    };

    // A link inside the tree was not copied, so removing the source would
    // destroy it — the copy would silently turn a link into nothing, and a
    // restore could never put it back. Refuse the whole move and leave the
    // original exactly where it is (security review M7).
    if !skipped_links.is_empty() {
        cleanup_partial(to, created_destination);
        return Err(CoreError::Quarantine(format!(
            "`{}` contains links that cannot be copied to another volume ({}); \
             it was left where it is",
            from.display(),
            skipped_links
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let verified = if metadata.is_dir() {
        dirs_are_identical(from, to)
    } else {
        copies_match(from, to)
    };
    if !verified.unwrap_or(false) {
        cleanup_partial(to, created_destination);
        return Err(CoreError::VerifyFailed(from.display().to_string()));
    }

    let removed = if metadata.is_dir() {
        fs::remove_dir_all(from)
    } else {
        fs::remove_file(from)
    };
    removed.map_err(|err| {
        CoreError::Quarantine(format!(
            "copied `{}` but could not remove the original: {err}",
            from.display()
        ))
    })
}

/// Remove a destination this call created, and nothing else.
///
/// It used to be an unguarded `remove_dir_all(to)`, and on the restore path
/// `to` is a user path: a copy that failed onto an existing folder deleted the
/// folder (security review H1). `created_by_this_call` comes from the copy
/// itself — from the `create_new` that succeeded, or the `create_dir` that did
/// — never from a probe taken beforehand, because the answer can change in
/// between (security review M10).
#[allow(clippy::disallowed_methods)]
fn cleanup_partial(path: &Path, created_by_this_call: bool) {
    if !created_by_this_call {
        tracing::warn!(
            path = %path.display(),
            "leaving the destination alone after a failed copy: this call did not create it"
        );
        return;
    }

    let removed = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    };
    if let Err(err) = removed {
        tracing::error!(
            path = %path.display(),
            error = %err,
            "could not clean up a half-written copy; it is left behind rather than retried blindly"
        );
    }
}

/// Copy one file to a destination that must not already exist, keeping its
/// timestamps and its read-only flag.
///
/// `fs::copy` would happily truncate whatever is at `to`, and it is the
/// exclusive claim on `to` that makes a restore safe (security review H2).
/// A cross-volume copy that lost the timestamps would also make a restored file
/// look brand new to every backup and every age-based rule (M6).
///
/// `created_destination` is set the instant `to` comes into existence, and is
/// what decides whether a failed copy may be cleaned up. It is an out-parameter
/// rather than part of the return value because the caller needs the answer on
/// the error path too (security review M10).
fn copy_file(from: &Path, to: &Path, created_destination: &mut bool) -> std::io::Result<()> {
    let metadata = fs::metadata(from)?;

    {
        let mut source = fs::File::open(from)?;
        let mut destination = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        // `create_new` succeeded, so nothing was at `to` and this call is what
        // put something there. Anything that fails from here on may be cleaned
        // up; anything that failed before this line may not.
        *created_destination = true;
        std::io::copy(&mut source, &mut destination)?;
        destination.sync_all()?;
    }

    let modified = metadata.modified()?;
    let times = fs::FileTimes::new()
        .set_modified(modified)
        .set_accessed(metadata.accessed().unwrap_or(modified));
    fs::File::options().write(true).open(to)?.set_times(times)?;

    // Last, because a read-only file cannot be opened for writing and the
    // timestamps above need exactly that.
    if metadata.permissions().readonly() {
        let mut permissions = fs::metadata(to)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(to, permissions)?;
    }
    Ok(())
}

/// Copy a directory tree, returning the links it refused to follow.
///
/// A link is never copied: following it would store the data it points at and
/// destroy the link on the way back, and recreating it needs a privilege this
/// app does not ask for. The caller turns a non-empty list into a refusal
/// (security review M7).
///
/// `created_destination` follows the same contract as [`copy_file`]'s: set the
/// instant `to` exists, and only for the destination handed to *this* call.
fn copy_dir_recursive(
    from: &Path,
    to: &Path,
    created_destination: &mut bool,
) -> std::io::Result<Vec<PathBuf>> {
    let mut skipped = Vec::new();
    // `create_dir` fails rather than succeeding on an existing directory, so
    // reaching the next line means this call made it.
    fs::create_dir(to)?;
    *created_destination = true;

    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;

        // Only the top-level destination decides what the cleanup may touch;
        // everything below it is already inside a directory this call owns, so
        // the children's flags are discarded.
        let mut child_created = false;
        if kind.is_symlink() {
            skipped.push(source);
        } else if kind.is_dir() {
            skipped.extend(copy_dir_recursive(&source, &target, &mut child_created)?);
        } else {
            copy_file(&source, &target, &mut child_created)?;
        }
    }
    Ok(skipped)
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
/// same read-only flag.
///
/// Bytes alone used to be the whole test, which passed a copy that had silently
/// dropped every timestamp and attribute the original carried
/// (security review M6).
fn copies_match(a: &Path, b: &Path) -> std::io::Result<bool> {
    if !files_are_identical(a, b)? {
        return Ok(false);
    }

    let (meta_a, meta_b) = (fs::metadata(a)?, fs::metadata(b)?);
    if meta_a.permissions().readonly() != meta_b.permissions().readonly() {
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

fn dirs_are_identical(a: &Path, b: &Path) -> std::io::Result<bool> {
    let names = |dir: &Path| -> std::io::Result<Vec<std::ffi::OsString>> {
        let mut names: Vec<_> = fs::read_dir(dir)?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        names.sort();
        Ok(names)
    };

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
        })
        .expect("serialize");
        assert_eq!(json, r#"{"itemCount":2,"totalBytes":9,"unreadableRows":0}"#);

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
            Err(MoveFailure::Failed(err)) => panic!("wrong failure: {err}"),
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
        ];

        fn visit(path: &Path, offenders: &mut Vec<String>) {
            if path.is_dir() {
                let Ok(entries) = fs::read_dir(path) else {
                    return;
                };
                for entry in entries.flatten() {
                    visit(&entry.path(), offenders);
                }
                return;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                return;
            }
            // This file is the sanctioned exception.
            if path.file_name().and_then(|n| n.to_str()) == Some("quarantine.rs") {
                return;
            }
            let Ok(source) = fs::read_to_string(path) else {
                return;
            };
            for (number, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if FORBIDDEN.iter().any(|needle| line.contains(needle)) {
                    offenders.push(format!("{}:{}", path.display(), number + 1));
                }
            }
        }

        // Read-only, and bounded to this crate's own sources.
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut offenders = Vec::new();
        for target in ["src", "tests", "build.rs"] {
            visit(&crate_root.join(target), &mut offenders);
        }

        assert!(
            offenders.is_empty(),
            "user data may only be removed by core/quarantine.rs; found: {offenders:?}"
        );
    }
}
