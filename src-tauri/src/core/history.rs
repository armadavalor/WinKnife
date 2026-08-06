//! What each module last did, so the dashboard shows facts rather than blanks.
//!
//! # Why this is separate from `state::ScanStore`
//!
//! The store holds *findings* — the paths a scan proposed acting on — and it is
//! deliberately in memory and capped, because a finding is a claim about a
//! filesystem that changes underneath it. Offering to act on last week's
//! snapshot is the defect that store exists to prevent.
//!
//! A dashboard card needs something much weaker and much longer-lived: "the
//! cleaner last ran on Tuesday, found 412 things and could free about 2 GB". No
//! path, nothing actionable, and it should survive a restart — so it goes in the
//! `scans` table, which ARCHITECTURE section 5 has described since phase 0 and
//! which nothing had written to until now.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::Result;

/// One line of a module's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRecord {
    /// Which module ran.
    pub module: String,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// When it finished, when it did.
    pub finished_at: Option<DateTime<Utc>>,
    /// How many things it found.
    pub finding_count: u64,
    /// Roughly how much it could reclaim. An estimate from the scan, and shown
    /// as one.
    pub reclaimable_bytes: u64,
    /// `done` or `cancelled`.
    pub status: String,
}

/// A scan that finished normally.
pub const STATUS_DONE: &str = "done";
/// A scan the user stopped.
pub const STATUS_CANCELLED: &str = "cancelled";

/// Write one line of history.
///
/// Failures are the caller's to log and swallow: a dashboard card that cannot
/// be written must never fail the scan that produced it.
///
/// # Errors
/// Returns an error if the row cannot be written.
pub fn record(
    conn: &Connection,
    scan_id: Uuid,
    module: &str,
    started_at: DateTime<Utc>,
    finding_count: u64,
    reclaimable_bytes: u64,
    cancelled: bool,
) -> Result<()> {
    conn.execute(
        "INSERT INTO scans
            (id, module, started_at, finished_at, finding_count, status, reclaimable_bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            scan_id.to_string(),
            module,
            started_at.to_rfc3339(),
            Utc::now().to_rfc3339(),
            finding_count as i64,
            if cancelled {
                STATUS_CANCELLED
            } else {
                STATUS_DONE
            },
            reclaimable_bytes as i64,
        ],
    )?;
    prune(conn, module)
}

/// How many runs of one module are kept.
///
/// The dashboard shows the latest; the rest are here so a future "history" view
/// has something to show, and so the table cannot grow without bound on a
/// machine somebody scans every day for a year.
pub const KEEP_PER_MODULE: usize = 20;

fn prune(conn: &Connection, module: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM scans
         WHERE module = ?1 AND id NOT IN (
             SELECT id FROM scans WHERE module = ?1
             ORDER BY started_at DESC LIMIT ?2
         )",
        rusqlite::params![module, KEEP_PER_MODULE as i64],
    )?;
    Ok(())
}

/// The most recent run of one module.
///
/// # Errors
/// Returns an error if the table cannot be read.
pub fn latest(conn: &Connection, module: &str) -> Result<Option<ScanRecord>> {
    let row = conn
        .query_row(
            "SELECT module, started_at, finished_at, finding_count, status, reclaimable_bytes
             FROM scans WHERE module = ?1 ORDER BY started_at DESC LIMIT 1",
            [module],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(to_record))
}

fn to_record(
    (module, started, finished, count, status, bytes): (
        String,
        String,
        Option<String>,
        i64,
        String,
        i64,
    ),
) -> Option<ScanRecord> {
    Some(ScanRecord {
        module,
        started_at: parse(&started)?,
        finished_at: finished.as_deref().and_then(parse),
        finding_count: count.max(0) as u64,
        reclaimable_bytes: bytes.max(0) as u64,
        status,
    })
}

fn parse(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|when| when.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn conn() -> Connection {
        db::open_in_memory().expect("db")
    }

    fn write(conn: &Connection, module: &str, when: DateTime<Utc>, count: u64, bytes: u64) {
        record(conn, Uuid::new_v4(), module, when, count, bytes, false).expect("record");
    }

    #[test]
    fn a_module_with_no_history_reads_as_none() {
        assert_eq!(latest(&conn(), "cleaner").expect("read"), None);
    }

    #[test]
    fn the_latest_run_is_the_one_returned() {
        let conn = conn();
        let old = Utc::now() - chrono::Duration::days(2);
        write(&conn, "cleaner", old, 10, 1_000);
        write(&conn, "cleaner", Utc::now(), 42, 9_999);

        let record = latest(&conn, "cleaner").expect("read").expect("a record");
        assert_eq!(record.finding_count, 42);
        assert_eq!(record.reclaimable_bytes, 9_999);
        assert_eq!(record.status, STATUS_DONE);
        assert!(record.finished_at.is_some());
    }

    #[test]
    fn one_modules_history_does_not_answer_for_another() {
        let conn = conn();
        write(&conn, "cleaner", Utc::now(), 5, 100);
        assert!(latest(&conn, "duplicates").expect("read").is_none());
    }

    #[test]
    fn a_cancelled_run_is_recorded_as_cancelled() {
        let conn = conn();
        record(&conn, Uuid::new_v4(), "treemap", Utc::now(), 0, 0, true).expect("record");
        assert_eq!(
            latest(&conn, "treemap")
                .expect("read")
                .expect("record")
                .status,
            STATUS_CANCELLED
        );
    }

    #[test]
    fn history_is_capped_per_module() {
        let conn = conn();
        for index in 0..(KEEP_PER_MODULE + 7) {
            write(
                &conn,
                "cleaner",
                Utc::now() - chrono::Duration::minutes(index as i64),
                index as u64,
                0,
            );
        }
        let kept: i64 = conn
            .query_row(
                "SELECT count(*) FROM scans WHERE module = 'cleaner'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(kept as usize, KEEP_PER_MODULE);
        assert_eq!(
            latest(&conn, "cleaner")
                .expect("read")
                .expect("record")
                .finding_count,
            0,
            "the newest row is index 0, which is the one with no minutes subtracted"
        );
    }

    #[test]
    fn records_serialise_in_camel_case() {
        let conn = conn();
        write(&conn, "cleaner", Utc::now(), 1, 2);
        let json = serde_json::to_value(latest(&conn, "cleaner").expect("read").expect("record"))
            .expect("serialize");
        assert!(json.get("reclaimableBytes").is_some(), "got {json}");
        assert!(json.get("findingCount").is_some(), "got {json}");
    }
}
