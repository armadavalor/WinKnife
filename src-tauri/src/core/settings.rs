//! Typed access to the `settings` key/value table.
//!
//! Why typed helpers rather than raw SQL at the call sites: the settings table
//! is a `TEXT` blob store, and the shutdown timer wants to persist a struct
//! across an app restart. One place that knows how to round-trip JSON keeps the
//! keys spelled consistently and the failures uniform.
//!
//! A malformed stored value is treated as "not set" and logged, never as a
//! startup failure — the app must still open when a setting is corrupt.

use rusqlite::{Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::core::error::Result;

/// The timer that should be re-armed on the next app start.
pub const KEY_ACTIVE_TIMER: &str = "shutdown-timer.active";

/// Quarantine retention window, in days.
pub const KEY_RETENTION_DAYS: &str = "quarantine.retention_days";

/// Quarantine size cap, in bytes.
pub const KEY_MAX_BYTES: &str = "quarantine.max_bytes";

/// Prefix under which each module stores its own settings blob.
///
/// One row per module rather than one row per field: a module owns the shape of
/// its own settings, and adding a field to it must not need a migration
/// (ARCHITECTURE section 5, "`settings(key, value)` — includes per-module
/// settings JSON").
pub const MODULE_KEY_PREFIX: &str = "module.";

/// The settings key for one module.
pub fn module_key(module_id: &str) -> String {
    format!("{MODULE_KEY_PREFIX}{module_id}")
}

/// Read a module's settings, or `None` when it has never saved any.
///
/// A stored blob that no longer deserialises reads as `None` and is logged, so
/// an upgrade that changed a field falls back to defaults instead of refusing to
/// start (see [`get_json`]).
///
/// # Errors
/// Returns an error if the table cannot be queried.
pub fn get_module<T: DeserializeOwned>(conn: &Connection, module_id: &str) -> Result<Option<T>> {
    get_json(conn, &module_key(module_id))
}

/// Write a module's settings.
///
/// # Errors
/// Returns an error if serialisation or the write fails.
pub fn set_module<T: Serialize>(conn: &Connection, module_id: &str, value: &T) -> Result<()> {
    set_json(conn, &module_key(module_id), value)
}

/// Read a raw string setting.
///
/// # Errors
/// Returns an error if the table cannot be queried.
pub fn get_raw(conn: &Connection, key: &str) -> Result<Option<String>> {
    let value = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()?;
    Ok(value)
}

/// Write a raw string setting.
///
/// # Errors
/// Returns an error if the write fails.
pub fn set_raw(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// Remove a setting. Removing something that is not there is fine.
///
/// # Errors
/// Returns an error if the delete fails.
pub fn clear(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
    Ok(())
}

/// Read a JSON-encoded setting.
///
/// A value that no longer deserialises (an older app version wrote it) is
/// reported as absent and logged, so an upgrade never bricks startup.
///
/// # Errors
/// Returns an error if the table cannot be queried.
pub fn get_json<T: DeserializeOwned>(conn: &Connection, key: &str) -> Result<Option<T>> {
    let Some(raw) = get_raw(conn, key)? else {
        return Ok(None);
    };
    match serde_json::from_str(&raw) {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            tracing::warn!(key, error = %err, "discarding an unreadable setting");
            Ok(None)
        }
    }
}

/// Write a JSON-encoded setting.
///
/// # Errors
/// Returns an error if serialisation or the write fails.
pub fn set_json<T: Serialize>(conn: &Connection, key: &str, value: &T) -> Result<()> {
    let raw = serde_json::to_string(value)?;
    set_raw(conn, key, &raw)
}

/// Read a `u64` setting, falling back to `default` when unset or unreadable.
///
/// # Errors
/// Returns an error if the table cannot be queried.
pub fn get_u64(conn: &Connection, key: &str, default: u64) -> Result<u64> {
    Ok(get_raw(conn, key)?
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(default))
}

/// Read an `i64` setting, falling back to `default`.
///
/// # Errors
/// Returns an error if the table cannot be queried.
pub fn get_i64(conn: &Connection, key: &str, default: i64) -> Result<i64> {
    Ok(get_raw(conn, key)?
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(default))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Sample {
        label: String,
        count: u32,
    }

    fn conn() -> Connection {
        db::open_in_memory().expect("db")
    }

    #[test]
    fn a_missing_key_reads_as_none() {
        assert_eq!(get_raw(&conn(), "nope").expect("read"), None);
    }

    #[test]
    fn raw_values_round_trip() {
        let conn = conn();
        set_raw(&conn, "theme", "dark").expect("write");
        assert_eq!(
            get_raw(&conn, "theme").expect("read"),
            Some("dark".to_owned())
        );
    }

    #[test]
    fn writing_the_same_key_twice_replaces_the_value() {
        let conn = conn();
        set_raw(&conn, "theme", "dark").expect("write");
        set_raw(&conn, "theme", "light").expect("overwrite");
        assert_eq!(
            get_raw(&conn, "theme").expect("read"),
            Some("light".to_owned())
        );
    }

    #[test]
    fn json_values_round_trip() {
        let conn = conn();
        let value = Sample {
            label: "timer".to_owned(),
            count: 3,
        };
        set_json(&conn, KEY_ACTIVE_TIMER, &value).expect("write");
        assert_eq!(
            get_json::<Sample>(&conn, KEY_ACTIVE_TIMER).expect("read"),
            Some(value)
        );
    }

    #[test]
    fn an_unreadable_json_value_is_discarded_rather_than_fatal() {
        let conn = conn();
        set_raw(&conn, KEY_ACTIVE_TIMER, "{ this is not the struct }").expect("write");
        assert_eq!(
            get_json::<Sample>(&conn, KEY_ACTIVE_TIMER).expect("read"),
            None,
            "a corrupt setting must not stop the app starting"
        );
    }

    #[test]
    fn clearing_removes_the_key() {
        let conn = conn();
        set_raw(&conn, "theme", "dark").expect("write");
        clear(&conn, "theme").expect("clear");
        assert_eq!(get_raw(&conn, "theme").expect("read"), None);
        clear(&conn, "theme").expect("clearing twice is fine");
    }

    #[test]
    fn module_settings_are_namespaced_and_round_trip() {
        let conn = conn();
        assert_eq!(module_key("cleaner"), "module.cleaner");

        let value = Sample {
            label: "packs".to_owned(),
            count: 6,
        };
        set_module(&conn, "cleaner", &value).expect("write");
        assert_eq!(
            get_module::<Sample>(&conn, "cleaner").expect("read"),
            Some(value)
        );
        assert_eq!(
            get_module::<Sample>(&conn, "duplicates").expect("read"),
            None,
            "one module's settings must not answer for another's"
        );
    }

    #[test]
    fn module_settings_that_no_longer_parse_fall_back_rather_than_fail() {
        let conn = conn();
        set_raw(&conn, &module_key("cleaner"), "{ from an older version }").expect("write");
        assert_eq!(
            get_module::<Sample>(&conn, "cleaner").expect("read"),
            None,
            "an upgrade must not brick the settings page"
        );
    }

    #[test]
    fn numeric_settings_fall_back_to_their_default() {
        let conn = conn();
        assert_eq!(get_u64(&conn, KEY_MAX_BYTES, 42).expect("read"), 42);
        assert_eq!(get_i64(&conn, KEY_RETENTION_DAYS, 7).expect("read"), 7);

        set_raw(&conn, KEY_MAX_BYTES, "1024").expect("write");
        assert_eq!(get_u64(&conn, KEY_MAX_BYTES, 42).expect("read"), 1024);

        set_raw(&conn, KEY_MAX_BYTES, "not a number").expect("write");
        assert_eq!(
            get_u64(&conn, KEY_MAX_BYTES, 42).expect("read"),
            42,
            "a garbage value must fall back, not panic"
        );
    }
}
