//! SQLite storage: scan history, executed actions, quarantine ledger, settings.
//!
//! Why: the quarantine ledger is the only thing that knows where a moved file
//! came from, so persistence has to be boring and durable. Migrations are
//! numbered SQL files embedded with `include_str!` and applied at startup;
//! `pragma user_version` records how far we got (ARCHITECTURE section 5).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::core::error::{CoreError, Result};

/// Database file name inside the application data directory.
pub const DB_FILE_NAME: &str = "winknife.sqlite3";

/// One numbered migration.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// Schema version this migration produces. Must equal its 1-based index.
    pub version: u32,
    /// Short slug, used in error messages only.
    pub name: &'static str,
    /// The SQL, applied as one transaction.
    pub sql: &'static str,
}

/// Every migration, in order. Append only — never edit a released entry.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../../migrations/001_init.sql"),
    },
    Migration {
        version: 2,
        name: "treemap_cache",
        sql: include_str!("../../migrations/002_treemap_cache.sql"),
    },
    Migration {
        version: 3,
        name: "scan_history",
        sql: include_str!("../../migrations/003_scan_history.sql"),
    },
    Migration {
        version: 4,
        name: "quarantine_restore_refused",
        sql: include_str!("../../migrations/004_quarantine_restore_refused.sql"),
    },
    Migration {
        version: 5,
        name: "tweak_undo",
        sql: include_str!("../../migrations/005_tweak_undo.sql"),
    },
];

/// The schema version the current build expects.
pub fn target_version() -> u32 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

/// Where the database lives for a given application data directory.
pub fn db_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DB_FILE_NAME)
}

/// Open (creating if needed) the database inside `app_data_dir`, apply pending
/// migrations and return the connection.
///
/// # Errors
/// Returns an error if the directory cannot be created, the file cannot be
/// opened, or a migration fails.
pub fn open(app_data_dir: &Path) -> Result<Connection> {
    std::fs::create_dir_all(app_data_dir)?;
    let mut conn = Connection::open(db_path(app_data_dir))?;
    prepare(&mut conn)?;
    Ok(conn)
}

/// Open an in-memory database with the full schema. For tests.
///
/// # Errors
/// Returns an error if a migration fails.
pub fn open_in_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    prepare(&mut conn)?;
    Ok(conn)
}

/// Apply the connection pragmas and run pending migrations.
///
/// # Errors
/// Returns an error if a pragma or a migration fails.
pub fn prepare(conn: &mut Connection) -> Result<()> {
    // Cascading deletes only work with foreign keys switched on, and sqlite
    // defaults them off per connection.
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // WAL survives an abrupt shutdown better, which matters for a ledger.
    let _journal_mode: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    run_migrations(conn)?;
    Ok(())
}

/// Read `pragma user_version`.
///
/// # Errors
/// Returns an error if the pragma cannot be read.
pub fn schema_version(conn: &Connection) -> Result<u32> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version.max(0) as u32)
}

/// Apply every migration newer than the stored schema version.
///
/// Each migration runs in its own transaction together with the version bump,
/// so an interrupted upgrade never leaves a half-applied schema. Returns the
/// version the database is at afterwards.
///
/// # Errors
/// Returns [`CoreError::SchemaTooNew`] without touching the database if it
/// was written by a newer build (round 3: an older build does not know the
/// restore-refused marker and would purge what the newer one keeps), or an
/// error if a migration's SQL fails.
pub fn run_migrations(conn: &mut Connection) -> Result<u32> {
    let mut current = schema_version(conn)?;
    let known = MIGRATIONS.last().map_or(0, |m| m.version);
    if current > known {
        return Err(CoreError::SchemaTooNew {
            found: current,
            known,
        });
    }

    for migration in MIGRATIONS {
        debug_assert_eq!(
            migration.version,
            MIGRATIONS
                .iter()
                .position(|m| m.version == migration.version)
                .map_or(0, |i| i as u32 + 1),
            "migration versions must be consecutive and 1-based"
        );

        if migration.version <= current {
            continue;
        }

        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        // `user_version` is not parameterisable, but the value is a typed u32
        // from a compile-time constant, so there is nothing to inject.
        tx.pragma_update(None, "user_version", migration.version)?;
        tx.commit()?;
        current = migration.version;
    }

    Ok(current)
}

/// Tauri-managed handle around the single connection.
///
/// rusqlite connections are not `Sync`, so access is serialised through a
/// mutex. Long-running work is expected to run on `spawn_blocking` anyway.
#[derive(Debug)]
pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    /// Wrap an already-migrated connection.
    pub fn new(connection: Connection) -> Self {
        Self {
            connection: Mutex::new(connection),
        }
    }

    /// Run `f` with exclusive access to the connection.
    ///
    /// # Panics
    /// Panics if another thread panicked while holding the lock; at that point
    /// the connection's state is unknown and continuing would be worse.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&guard)
    }
}

/// Something that can lend out the connection for one short piece of work.
///
/// Long operations (a clean, a restore, a purge, a startup scan) take this
/// instead of a `&Connection` so they can lock per statement rather than for
/// their whole run: the filesystem work between two ledger writes then happens
/// with the lock released, and the drawer, the settings and the treemap stay
/// responsive meanwhile (audit issue #37). A bare `Connection` implements it
/// too, so tests and single-threaded start-up code keep passing one directly.
///
/// Object-safe on purpose; [`with_db`] is the typed way to call it.
pub trait DbAccess {
    /// Run `f` with the connection, holding whatever lock guards it only for
    /// the length of the call.
    ///
    /// # Errors
    /// Returns whatever `f` returned.
    fn access(&self, f: &mut dyn FnMut(&Connection) -> Result<()>) -> Result<()>;
}

impl DbAccess for Connection {
    fn access(&self, f: &mut dyn FnMut(&Connection) -> Result<()>) -> Result<()> {
        f(self)
    }
}

impl DbAccess for Database {
    fn access(&self, f: &mut dyn FnMut(&Connection) -> Result<()>) -> Result<()> {
        self.with(|conn| f(conn))
    }
}

/// Run `f` through `db` and hand back its value.
///
/// # Errors
/// Returns whatever `f` returned, or [`CoreError::Db`] if `db` never called
/// it (no implementation here does that).
pub fn with_db<T>(db: &dyn DbAccess, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let mut f = Some(f);
    let mut out = None;
    db.access(&mut |conn| {
        if let Some(f) = f.take() {
            out = Some(f(conn)?);
        }
        Ok(())
    })?;
    out.ok_or(CoreError::Db(rusqlite::Error::InvalidQuery))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .expect("prepare");
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect");
        names
    }

    #[test]
    fn migrations_are_consecutive_and_one_based() {
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                migration.version,
                index as u32 + 1,
                "migration `{}` is out of order",
                migration.name
            );
        }
        assert_eq!(target_version(), MIGRATIONS.len() as u32);
    }

    #[test]
    fn fresh_database_reaches_the_target_version() {
        let mut conn = Connection::open_in_memory().expect("open");
        assert_eq!(schema_version(&conn).expect("version"), 0);
        let version = run_migrations(&mut conn).expect("migrate");
        assert_eq!(version, target_version());
        assert_eq!(schema_version(&conn).expect("version"), target_version());
    }

    #[test]
    fn migration_001_creates_the_architecture_tables() {
        let conn = open_in_memory().expect("open");
        assert_eq!(
            table_names(&conn),
            vec![
                "actions",
                "quarantine",
                "scans",
                "settings",
                "startup_disabled_link",
                "treemap_dir",
                "treemap_scan",
                "tweak_undo",
            ]
        );
    }

    #[test]
    fn run_migrations_is_idempotent() {
        let mut conn = Connection::open_in_memory().expect("open");
        run_migrations(&mut conn).expect("first run");
        let before = table_names(&conn);
        let version = run_migrations(&mut conn).expect("second run");
        assert_eq!(version, target_version());
        assert_eq!(table_names(&conn), before);
    }

    #[test]
    fn actions_cascade_when_their_scan_disappears() {
        let conn = open_in_memory().expect("open");
        conn.execute(
            "INSERT INTO scans (id, module, started_at, status)
             VALUES ('s1', 'dummy', '2026-08-06T00:00:00Z', 'done')",
            [],
        )
        .expect("insert scan");
        conn.execute(
            "INSERT INTO actions (id, scan_id, recommendation_json)
             VALUES ('a1', 's1', '{}')",
            [],
        )
        .expect("insert action");

        conn.execute("DELETE FROM scans WHERE id = 's1'", [])
            .expect("delete scan");

        let remaining: i64 = conn
            .query_row("SELECT count(*) FROM actions", [], |row| row.get(0))
            .expect("count");
        assert_eq!(remaining, 0, "foreign_keys pragma is not active");
    }

    #[test]
    fn quarantine_row_round_trips() {
        let conn = open_in_memory().expect("open");
        conn.execute(
            "INSERT INTO quarantine
                (id, batch_id, original_path, quarantine_path, size_bytes,
                 quarantined_at, purge_after)
             VALUES ('q1', 'b1', 'C:\\Temp\\a.log', 'C:\\qt\\b1\\q1', 42,
                     '2026-08-06T00:00:00Z', '2026-08-13T00:00:00Z')",
            [],
        )
        .expect("insert");

        let (original, size, restored): (String, i64, Option<String>) = conn
            .query_row(
                "SELECT original_path, size_bytes, restored_at FROM quarantine WHERE id = 'q1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("select");

        assert_eq!(original, "C:\\Temp\\a.log");
        assert_eq!(size, 42);
        assert_eq!(restored, None);
    }

    /// Decision b (2026-10-12): migration 4 adds a nullable column, so a
    /// database written by an earlier build upgrades in place and its held
    /// items load unmarked.
    #[test]
    fn a_version_3_database_gains_the_restore_refused_marker() {
        let mut conn = Connection::open_in_memory().expect("open");
        for migration in MIGRATIONS.iter().take_while(|m| m.version <= 3) {
            conn.execute_batch(migration.sql).expect("old schema");
        }
        conn.pragma_update(None, "user_version", 3)
            .expect("old version");
        let id = uuid::Uuid::new_v4().to_string();
        let batch = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO quarantine
                (id, batch_id, original_path, quarantine_path, size_bytes,
                 quarantined_at, purge_after)
             VALUES (?1, ?2, 'a.log', 'q', 42,
                     '2026-08-06T00:00:00.000Z', '2026-08-13T00:00:00.000Z')",
            rusqlite::params![id, batch],
        )
        .expect("old row");

        assert_eq!(
            run_migrations(&mut conn).expect("upgrade"),
            target_version()
        );

        let quarantine = crate::core::quarantine::Quarantine::with_root(
            std::env::temp_dir().join("winknife-unused"),
            crate::core::quarantine::RetentionPolicy::default(),
        );
        let item = quarantine
            .find_item(&conn, uuid::Uuid::parse_str(&id).expect("uuid"))
            .expect("read")
            .expect("the old row still loads");
        assert_eq!(item.size_bytes, 42);
        assert!(item.is_held());
        assert_eq!(item.restore_refused_at, None);
    }

    /// Round 3: a database written by a newer build is refused, not opened.
    /// An older build does not know the restore-refused marker, so it would
    /// let the automatic purge remove items the newer one promised to keep.
    #[test]
    fn a_database_from_a_newer_build_is_refused() {
        let mut conn = Connection::open_in_memory().expect("open");
        let known = run_migrations(&mut conn).expect("current schema");
        let newer = known + 1;
        conn.pragma_update(None, "user_version", newer)
            .expect("newer version");

        let err = run_migrations(&mut conn).expect_err("a newer schema is refused");

        assert!(
            matches!(err, CoreError::SchemaTooNew { found, known: k } if found == newer && k == known),
            "got {err}"
        );
        let text = err.to_string();
        assert!(text.contains(&newer.to_string()), "{text}");
        assert!(text.contains(&known.to_string()), "{text}");
        assert_eq!(
            schema_version(&conn).expect("version"),
            newer,
            "left untouched"
        );
    }

    #[test]
    fn strict_tables_reject_wrong_column_types() {
        let conn = open_in_memory().expect("open");
        let err = conn.execute(
            "INSERT INTO quarantine
                (id, batch_id, original_path, quarantine_path, size_bytes,
                 quarantined_at, purge_after)
             VALUES ('q2', 'b1', 'p', 'q', 'not-a-number', 'now', 'later')",
            [],
        );
        assert!(err.is_err(), "STRICT should reject a text size_bytes");
    }

    #[test]
    fn open_creates_the_file_and_parent_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_data = dir.path().join("nested").join("winknife");
        let conn = open(&app_data).expect("open");

        assert!(db_path(&app_data).is_file());
        assert_eq!(schema_version(&conn).expect("version"), target_version());
    }

    #[test]
    fn reopening_an_existing_database_does_not_re_run_migrations() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_data = dir.path().to_path_buf();

        {
            let conn = open(&app_data).expect("first open");
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('theme', 'dark')",
                [],
            )
            .expect("insert setting");
        }

        let conn = open(&app_data).expect("second open");
        let value: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'theme'",
                [],
                |row| row.get(0),
            )
            .expect("select");
        assert_eq!(value, "dark", "existing data must survive a reopen");
    }

    #[test]
    fn database_handle_serialises_access() {
        let db = Database::new(open_in_memory().expect("open"));
        let count = db
            .with(|conn| {
                let n: i64 =
                    conn.query_row("SELECT count(*) FROM settings", [], |row| row.get(0))?;
                Ok(n)
            })
            .expect("query through handle");
        assert_eq!(count, 0);
    }
}
