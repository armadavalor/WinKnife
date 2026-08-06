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

/// Buffer used when copying and verifying across volumes.
const COPY_BUFFER_BYTES: usize = 64 * 1024;

/// Suffix appended when a restore would overwrite something.
pub const RESTORE_COLLISION_SUFFIX: &str = " (restored)";

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
}

impl PurgeReport {
    /// Whether anything at all happened.
    pub fn is_empty(&self) -> bool {
        self.purged_items == 0 && self.failed_items.is_empty()
    }

    fn merge(&mut self, other: Self) {
        self.purged_items += other.purged_items;
        self.freed_bytes += other.freed_bytes;
        self.failed_items.extend(other.failed_items);
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
    pub fn new(app_data_dir: &Path, policy: RetentionPolicy) -> Self {
        Self {
            root: app_data_dir.join(QUARANTINE_DIR_NAME),
            policy,
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
            directory_size(path)
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

        if let Err(err) = move_path(path, &destination) {
            // The move failed, so the row describes something that never
            // happened. Drop it rather than leaving a phantom in the drawer.
            let _ = conn.execute(
                "DELETE FROM quarantine WHERE id = ?1",
                [item_id.to_string()],
            );
            return Err(err);
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

        let (target, renamed) = non_colliding_target(&item.original_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                CoreError::Quarantine(format!(
                    "cannot recreate `{}` for the restore: {err}",
                    parent.display()
                ))
            })?;
        }

        move_path(&item.quarantine_path, &target)?;

        let now = Utc::now();
        conn.execute(
            "UPDATE quarantine SET restored_at = ?2 WHERE id = ?1",
            rusqlite::params![item_id.to_string(), now.to_rfc3339()],
        )?;

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
        let expired = stmt
            .query_map([now.to_rfc3339()], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

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
        let candidates = stmt
            .query_map([], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut doomed = Vec::new();
        for item in candidates {
            if over_by == 0 {
                break;
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
        let items = stmt
            .query_map([], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
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
        let items = stmt
            .query_map([batch.to_string()], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
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
                Ok(QuarantineBatch {
                    batch_id: parse_uuid(&batch_id),
                    item_count: row.get::<_, i64>(1)?.max(0) as u64,
                    size_bytes: row.get::<_, i64>(2)?.max(0) as u64,
                    quarantined_at: parse_time(&quarantined_at),
                    purge_after: parse_time(&purge_after),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(batches)
    }

    /// Totals across everything still held.
    ///
    /// # Errors
    /// Returns an error if the ledger cannot be read.
    pub fn stats(&self, conn: &Connection) -> Result<QuarantineStats> {
        let (item_count, total_bytes): (i64, i64) = conn.query_row(
            "SELECT count(*), coalesce(sum(size_bytes), 0)
             FROM quarantine WHERE restored_at IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(QuarantineStats {
            item_count: item_count.max(0) as u64,
            total_bytes: total_bytes.max(0) as u64,
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
            Some(row) => Ok(Some(row?)),
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

            let removed = if !item.quarantine_path.exists() {
                // Already gone (manual cleanup, previous half-finished purge).
                Ok(())
            } else if item.quarantine_path.is_dir() {
                fs::remove_dir_all(&item.quarantine_path)
            } else {
                fs::remove_file(&item.quarantine_path)
            };

            match removed {
                Ok(()) => {
                    conn.execute(
                        "DELETE FROM quarantine WHERE id = ?1",
                        [item.id.to_string()],
                    )?;
                    report.purged_items += 1;
                    report.freed_bytes += item.size_bytes;
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

    fn prune_empty_batch_dirs(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && fs::read_dir(&path).is_ok_and(|mut e| e.next().is_none()) {
                let _ = fs::remove_dir(&path);
            }
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
            item.quarantined_at.to_rfc3339(),
            item.purge_after.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuarantineItem> {
    let id: String = row.get(0)?;
    let batch_id: String = row.get(1)?;
    let original_path: String = row.get(2)?;
    let quarantine_path: String = row.get(3)?;
    let quarantined_at: String = row.get(5)?;
    let purge_after: String = row.get(6)?;
    let restored_at: Option<String> = row.get(7)?;

    Ok(QuarantineItem {
        id: parse_uuid(&id),
        batch_id: parse_uuid(&batch_id),
        original_path: PathBuf::from(original_path),
        quarantine_path: PathBuf::from(quarantine_path),
        size_bytes: row.get::<_, i64>(4)?.max(0) as u64,
        quarantined_at: parse_time(&quarantined_at),
        purge_after: parse_time(&purge_after),
        restored_at: restored_at.as_deref().map(parse_time),
    })
}

/// Ids and timestamps are written by this module and only ever read back here.
/// A corrupted value should not take the drawer down, so both parsers fall back
/// to a sentinel the UI can still render.
fn parse_uuid(raw: &str) -> Uuid {
    Uuid::parse_str(raw).unwrap_or_else(|_| {
        tracing::error!(value = raw, "unparsable uuid in the quarantine ledger");
        Uuid::nil()
    })
}

fn parse_time(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| {
            tracing::error!(value = raw, "unparsable timestamp in the quarantine ledger");
            DateTime::UNIX_EPOCH
        })
}

/// Where to restore to when the original path is occupied.
fn non_colliding_target(original: &Path) -> (PathBuf, bool) {
    if !original.exists() {
        return (original.to_path_buf(), false);
    }

    let parent = original.parent().unwrap_or_else(|| Path::new(""));
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = original
        .extension()
        .map(|e| e.to_string_lossy().into_owned());

    for attempt in 0..1_000u32 {
        let mut name = stem.clone();
        name.push_str(RESTORE_COLLISION_SUFFIX);
        if attempt > 0 {
            name.push_str(&format!(" {}", attempt + 1));
        }
        if let Some(ext) = &ext {
            name.push('.');
            name.push_str(ext);
        }
        let candidate = parent.join(name);
        if !candidate.exists() {
            return (candidate, true);
        }
    }

    // Astronomically unlikely; fall back to a uuid rather than looping forever.
    (parent.join(Uuid::new_v4().to_string()), true)
}

/// Rename when we can, copy-verify-remove when we must.
fn move_path(from: &Path, to: &Path) -> Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            // A rename across volumes fails with ERROR_NOT_SAME_DEVICE (17) on
            // Windows; other errors (sharing violation, permissions) are real
            // and must not be papered over with a copy.
            let cross_volume = matches!(rename_err.raw_os_error(), Some(17) | Some(18))
                || cfg!(not(windows)) && rename_err.raw_os_error() == Some(18);
            if !cross_volume {
                return Err(CoreError::Quarantine(format!(
                    "cannot move `{}` to `{}`: {rename_err}",
                    from.display(),
                    to.display()
                )));
            }
            copy_verify_remove(from, to)
        }
    }
}

/// Cross-volume move: copy, prove the copy is identical, then remove the source.
///
/// The order matters. If verification fails the original is still there and the
/// half-written copy is cleaned up, so a bad move costs the user nothing.
fn copy_verify_remove(from: &Path, to: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(from).map_err(|err| {
        CoreError::Quarantine(format!("cannot inspect `{}`: {err}", from.display()))
    })?;

    let copied = if metadata.is_dir() {
        copy_dir_recursive(from, to)
    } else {
        fs::copy(from, to).map(|_| ())
    };
    if let Err(err) = copied {
        cleanup_partial(to);
        return Err(CoreError::Quarantine(format!(
            "cannot copy `{}` to `{}`: {err}",
            from.display(),
            to.display()
        )));
    }

    let verified = if metadata.is_dir() {
        dirs_are_identical(from, to)
    } else {
        files_are_identical(from, to)
    };
    if !verified.unwrap_or(false) {
        cleanup_partial(to);
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

fn cleanup_partial(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else if path.exists() {
        let _ = fs::remove_file(path);
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
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

fn dirs_are_identical(a: &Path, b: &Path) -> std::io::Result<bool> {
    let mut entries_a: Vec<_> = fs::read_dir(a)?
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name())
        .collect();
    let mut entries_b: Vec<_> = fs::read_dir(b)?
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name())
        .collect();
    entries_a.sort();
    entries_b.sort();
    if entries_a != entries_b {
        return Ok(false);
    }

    for name in entries_a {
        let (child_a, child_b) = (a.join(&name), b.join(&name));
        let same = if child_a.is_dir() {
            dirs_are_identical(&child_a, &child_b)?
        } else {
            files_are_identical(&child_a, &child_b)?
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Recursive size of a directory. Unreadable entries contribute zero rather
/// than failing the whole move.
fn directory_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => directory_size(&entry.path()),
            Ok(_) => entry.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
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
            // Creating symlinks needs developer mode or elevation on Windows.
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

    #[test]
    fn collision_naming_walks_up_until_it_finds_a_free_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = write(dir.path(), "doc.txt", b"a");
        write(dir.path(), "doc (restored).txt", b"b");

        let (target, renamed) = non_colliding_target(&original);
        assert!(renamed);
        assert_eq!(target, dir.path().join("doc (restored) 2.txt"));
    }

    #[test]
    fn collision_naming_handles_a_file_without_an_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = write(dir.path(), "LICENSE", b"a");
        let (target, renamed) = non_colliding_target(&original);
        assert!(renamed);
        assert_eq!(target, dir.path().join("LICENSE (restored)"));
    }

    #[test]
    fn wire_formats_match_typescript() {
        let json = serde_json::to_string(&QuarantineStats {
            item_count: 2,
            total_bytes: 9,
        })
        .expect("serialize");
        assert_eq!(json, r#"{"itemCount":2,"totalBytes":9}"#);

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

    /// CLAUDE.md hard rule, enforced against the source rather than trusted:
    /// only this file may permanently remove user data.
    #[test]
    fn no_other_module_removes_user_data() {
        fn visit(dir: &Path, offenders: &mut Vec<String>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit(&path, offenders);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                // This file is the sanctioned exception, and the tempdir
                // helpers in test modules clean up after themselves.
                if path.file_name().and_then(|n| n.to_str()) == Some("quarantine.rs") {
                    continue;
                }
                let Ok(source) = fs::read_to_string(&path) else {
                    continue;
                };
                for (number, line) in source.lines().enumerate() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") || trimmed.starts_with("///") {
                        continue;
                    }
                    if line.contains("fs::remove_file")
                        || line.contains("fs::remove_dir")
                        || line.contains("remove_dir_all")
                    {
                        offenders.push(format!("{}:{}", path.display(), number + 1));
                    }
                }
            }
        }

        // Read-only, and bounded to this crate's own sources.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        visit(&src, &mut offenders);

        assert!(
            offenders.is_empty(),
            "user data may only be removed by core/quarantine.rs; found: {offenders:?}"
        );
    }
}
