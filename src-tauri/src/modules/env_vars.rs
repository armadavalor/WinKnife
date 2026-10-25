//! Environment-variable editor (C15, K-024): list user and system variables,
//! edit the user ones with a diff preview first and an undo after.
//!
//! Every write goes through the C1 `tweak_undo` table (K-018): the old value
//! (or "absent") is recorded before the write, the write is read back, and
//! Undo puts back exactly what was there, `REG_SZ` vs `REG_EXPAND_SZ` included.
//! System variables live in `HKLM` and are only ever read.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::platform::env::{expand, Expansion};
use crate::core::platform::{
    broadcast_environment_change, EnvResolver, Hive, RegValue, Registry, REG_TYPE_EXPAND_SZ,
    REG_TYPE_SZ,
};
use crate::modules::hosts::{diff, DiffLine};

/// Prefix of this module's rows in `tweak_undo`.
pub const TWEAK_PREFIX: &str = "env:";
const USER_KEY: &str = "Environment";
const SYSTEM_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
/// Registry limit on a value name.
pub const MAX_NAME_CHARS: usize = 16_383;
/// Windows limit on one environment variable (32 767 including the NUL).
pub const MAX_VALUE_CHARS: usize = 32_766;
/// `applied_kind` of a delete: `REG_NONE` with no bytes means "absent after".
const APPLIED_ABSENT_KIND: u32 = 0;
const LIST_SEPARATOR: char = ';';

/// Where a variable lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// `HKCU\Environment`: editable.
    User,
    /// `HKLM\...\Session Manager\Environment`: read-only.
    System,
}

/// A variable's text and whether it is `REG_EXPAND_SZ`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvValue {
    /// The raw text, `%VAR%` unexpanded.
    pub value: String,
    /// `REG_EXPAND_SZ` when true, `REG_SZ` when false.
    pub expand: bool,
}

/// Why one entry of a `;` list looks wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FlagKind {
    /// Nothing between two separators.
    Empty,
    /// Same place as an earlier entry.
    Duplicate,
    /// The path does not exist.
    Missing,
    /// A `%VAR%` in it is not set.
    Unresolved,
}

/// One flag with its human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryFlag {
    /// What is wrong.
    pub kind: FlagKind,
    /// Why, in plain words.
    pub reason: String,
}

/// One entry of a `;` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathEntry {
    /// The entry as stored.
    pub text: String,
    /// After `%VAR%` expansion, when that changed something.
    pub expanded: Option<String>,
    /// Empty when the entry looks fine.
    pub flags: Vec<EntryFlag>,
}

/// One row of the listing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvVar {
    /// The name as stored.
    pub name: String,
    /// The raw text.
    pub value: String,
    /// `REG_EXPAND_SZ`.
    pub expand: bool,
    /// User or system.
    pub scope: Scope,
    /// Whether this page may change it.
    pub editable: bool,
    /// Why not, when it may not.
    pub read_only_reason: Option<String>,
    /// Holds `;`-separated entries.
    pub is_list: bool,
    /// The entries, when `is_list`.
    pub entries: Vec<PathEntry>,
}

/// A change WinKnife can undo.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoEntry {
    /// The variable name.
    pub name: String,
    /// The last change deleted it.
    pub deleted: bool,
}

/// What the page shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvListing {
    /// User variables, by name.
    pub user: Vec<EnvVar>,
    /// System variables, by name.
    pub system: Vec<EnvVar>,
    /// Changes with an open undo record.
    pub undo: Vec<UndoEntry>,
}

/// Add, edit or delete one variable.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvChange {
    /// Variable name.
    pub name: String,
    /// Must be `User`; `System` is always refused.
    pub scope: Scope,
    /// The new value, `None` to delete.
    pub new: Option<EnvValue>,
    /// What the user was shown, `None` when adding.
    pub expected: Option<EnvValue>,
}

/// Old → new, shown before the user confirms.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvPreview {
    /// The live value.
    pub before: Option<EnvValue>,
    /// The value after the change.
    pub after: Option<EnvValue>,
    /// Per entry for lists, per line otherwise.
    pub diff: Vec<DiffLine>,
}

/// Split a `;` list. [`join_path`] gives back the same text.
pub fn split_path(value: &str) -> Vec<String> {
    value.split(LIST_SEPARATOR).map(str::to_owned).collect()
}

/// Join entries back into one `;` list.
pub fn join_path(entries: &[String]) -> String {
    entries.join(&LIST_SEPARATOR.to_string())
}

/// Refuse a name or value the registry or Windows would not take.
///
/// # Errors
/// Empty name, `=` or NUL in the name, name or value over the limits, NUL in
/// the value.
pub fn validate(name: &str, value: Option<&EnvValue>) -> Result<()> {
    let refuse = |why: &str| Err(CoreError::Tweak(why.to_owned()));
    if name.trim().is_empty() {
        return refuse("the name is empty");
    }
    if name.contains('=') {
        return refuse("a name cannot contain `=`");
    }
    if name.contains('\0') {
        return refuse("a name cannot contain a NUL character");
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return refuse("the name is longer than the registry allows (16 383 characters)");
    }
    let Some(value) = value else { return Ok(()) };
    if value.value.contains('\0') {
        return refuse("a value cannot contain a NUL character");
    }
    if value.value.encode_utf16().count() > MAX_VALUE_CHARS {
        return refuse("the value is longer than Windows allows (32 766 characters)");
    }
    Ok(())
}

fn looks_like_path(text: &str) -> bool {
    text.contains(['\\', '/', ':', '%'])
}

fn place_key(text: &str) -> String {
    text.trim().trim_end_matches(['\\', '/']).to_lowercase()
}

/// The entries of a list value with their flags. `expand` follows the value's
/// kind: Windows only expands `%VAR%` in `REG_EXPAND_SZ`.
pub fn entries_of(
    value: &str,
    expand_vars: bool,
    env: &dyn EnvResolver,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<PathEntry> {
    let mut seen: Vec<String> = Vec::new();
    split_path(value)
        .into_iter()
        .map(|text| {
            let mut flags = Vec::new();
            let mut expanded = None;
            if text.trim().is_empty() {
                flags.push(EntryFlag {
                    kind: FlagKind::Empty,
                    reason: "empty entry; Windows skips it".to_owned(),
                });
                return PathEntry { text, expanded, flags };
            }
            let resolved = if expand_vars {
                match expand(&text, env) {
                    Expansion::Expanded(path) => Some(path),
                    Expansion::Missing(var) => {
                        flags.push(EntryFlag {
                            kind: FlagKind::Unresolved,
                            reason: format!("%{var}% is not set, so this entry points nowhere"),
                        });
                        None
                    }
                }
            } else {
                Some(text.clone())
            };
            if let Some(path) = &resolved {
                if path != &text {
                    expanded = Some(path.clone());
                }
                let key = place_key(path);
                // A duplicate is already flagged; its place was checked as the earlier entry.
                if let Some(index) = seen.iter().position(|earlier| *earlier == key) {
                    flags.push(EntryFlag {
                        kind: FlagKind::Duplicate,
                        reason: format!("same place as entry {}; this one adds nothing", index + 1),
                    });
                    return PathEntry { text, expanded, flags };
                }
                seen.push(key);
                if looks_like_path(&text) && !exists(Path::new(path.trim())) {
                    flags.push(EntryFlag {
                        kind: FlagKind::Missing,
                        reason: format!("`{}` does not exist", path.trim()),
                    });
                }
            }
            PathEntry { text, expanded, flags }
        })
        .collect()
}

fn text_of(value: &RegValue) -> Option<EnvValue> {
    value.as_string().map(|text| EnvValue {
        value: text,
        expand: value.kind == REG_TYPE_EXPAND_SZ,
    })
}

fn same_value(value: Option<&RegValue>, expected: &(u32, Vec<u8>)) -> bool {
    if expected.0 == APPLIED_ABSENT_KIND {
        return value.is_none();
    }
    value.is_some_and(|value| value.kind == expected.0 && value.bytes == expected.1)
}

fn tweak_id_of(name: &str) -> String {
    format!("{TWEAK_PREFIX}{}", name.to_uppercase())
}

struct UndoRow {
    id: i64,
    hive: String,
    value_name: String,
    prior: Option<(u32, Vec<u8>)>,
    applied: (u32, Vec<u8>),
}

fn open_row(conn: &Connection, tweak_id: &str) -> Result<Option<UndoRow>> {
    Ok(conn
        .query_row(
            "SELECT id, hive, value_name, prior_kind, prior_bytes, applied_kind, applied_bytes
             FROM tweak_undo
             WHERE tweak_id = ?1 AND closed_at IS NULL
             ORDER BY id DESC LIMIT 1",
            params![tweak_id],
            |row| {
                let prior_kind: Option<u32> = row.get(3)?;
                let prior_bytes: Option<Vec<u8>> = row.get(4)?;
                Ok(UndoRow {
                    id: row.get(0)?,
                    hive: row.get(1)?,
                    value_name: row.get(2)?,
                    prior: prior_kind.zip(prior_bytes),
                    applied: (row.get(5)?, row.get(6)?),
                })
            },
        )
        .optional()?)
}

fn close_row(conn: &Connection, id: i64, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE tweak_undo SET closed_at = ?2, close_reason = ?3 WHERE id = ?1",
        params![id, Utc::now().to_rfc3339(), reason],
    )?;
    Ok(())
}

fn open_undo(conn: &Connection) -> Result<Vec<UndoEntry>> {
    let mut stmt = conn.prepare(
        "SELECT value_name, applied_kind FROM tweak_undo
         WHERE substr(tweak_id, 1, 4) = ?1 AND closed_at IS NULL
         ORDER BY id",
    )?;
    let rows = stmt.query_map(params![TWEAK_PREFIX], |row| {
        Ok(UndoEntry {
            name: row.get(0)?,
            deleted: row.get::<_, u32>(1)? == APPLIED_ABSENT_KIND,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn diff_text(value: Option<&EnvValue>, as_list: bool) -> String {
    let text = value.map_or("", |value| value.value.as_str());
    if as_list {
        split_path(text).join("\n")
    } else {
        text.to_owned()
    }
}

/// Reads both scopes and writes the user one.
pub struct EnvVarService {
    registry: Arc<dyn Registry>,
}

impl std::fmt::Debug for EnvVarService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvVarService").finish_non_exhaustive()
    }
}

impl EnvVarService {
    /// Service over `registry`.
    pub fn new(registry: Arc<dyn Registry>) -> Self {
        Self { registry }
    }

    /// Both scopes plus the open undo records. Read-only.
    ///
    /// # Errors
    /// Registry or database read failures.
    pub fn list(
        &self,
        conn: &Connection,
        env: &dyn EnvResolver,
        exists: &dyn Fn(&Path) -> bool,
    ) -> Result<EnvListing> {
        Ok(EnvListing {
            user: self.read_scope(Scope::User, env, exists)?,
            system: self.read_scope(Scope::System, env, exists)?,
            undo: open_undo(conn)?,
        })
    }

    fn read_scope(
        &self,
        scope: Scope,
        env: &dyn EnvResolver,
        exists: &dyn Fn(&Path) -> bool,
    ) -> Result<Vec<EnvVar>> {
        let (hive, key) = match scope {
            Scope::User => (Hive::CurrentUser, USER_KEY),
            Scope::System => (Hive::LocalMachine, SYSTEM_KEY),
        };
        let mut vars: Vec<EnvVar> = self
            .registry
            .values(hive, key)?
            .into_iter()
            .filter(|value| !value.name.is_empty())
            .map(|raw| {
                let text = text_of(&raw);
                let read_only_reason = match (scope, &text) {
                    (Scope::System, _) => Some(
                        "system variable: changing it needs admin, so WinKnife only reads it"
                            .to_owned(),
                    ),
                    (Scope::User, None) => Some(
                        "not a text value (REG_SZ / REG_EXPAND_SZ); left as it is".to_owned(),
                    ),
                    (Scope::User, Some(_)) => None,
                };
                let EnvValue { value, expand } = text.unwrap_or(EnvValue {
                    value: String::new(),
                    expand: false,
                });
                let is_list = value.contains(LIST_SEPARATOR);
                let entries = if is_list {
                    entries_of(&value, expand, env, exists)
                } else {
                    Vec::new()
                };
                EnvVar {
                    name: raw.name,
                    value,
                    expand,
                    scope,
                    editable: read_only_reason.is_none(),
                    read_only_reason,
                    is_list,
                    entries,
                }
            })
            .collect();
        vars.sort_by_key(|var| var.name.to_lowercase());
        Ok(vars)
    }

    /// Every check before a write; returns the live value.
    fn check(&self, change: &EnvChange) -> Result<Option<RegValue>> {
        if change.scope != Scope::User {
            return Err(CoreError::Tweak(
                "system variables are read-only in WinKnife; nothing was written".to_owned(),
            ));
        }
        validate(&change.name, change.new.as_ref())?;
        let live = self.registry.value(Hive::CurrentUser, USER_KEY, &change.name)?;
        let shown = match &live {
            Some(raw) => Some(text_of(raw).ok_or_else(|| {
                CoreError::Tweak(format!("`{}` is not a text value; left as it is", change.name))
            })?),
            None => None,
        };
        if shown != change.expected {
            let why = if change.expected.is_none() {
                format!("`{}` already exists; edit it instead", change.name)
            } else {
                format!("`{}` changed since it was read; reload and try again", change.name)
            };
            return Err(CoreError::Tweak(why));
        }
        if shown == change.new {
            return Err(CoreError::Tweak("nothing to change".to_owned()));
        }
        Ok(live)
    }

    /// Old → new for the confirm dialog. Writes nothing.
    ///
    /// # Errors
    /// The same refusals as [`Self::apply`].
    pub fn preview(&self, change: &EnvChange) -> Result<EnvPreview> {
        self.check(change)?;
        let before = change.expected.clone();
        let after = change.new.clone();
        let as_list = [&before, &after]
            .iter()
            .any(|value| value.as_ref().is_some_and(|v| v.value.contains(LIST_SEPARATOR)));
        let diff = diff(&diff_text(before.as_ref(), as_list), &diff_text(after.as_ref(), as_list));
        Ok(EnvPreview { before, after, diff })
    }

    /// Record the old value, write, read back, tell running programs.
    ///
    /// # Errors
    /// Any refusal from the checks, a failed write, or a read-back mismatch
    /// (the old value is put back first).
    pub fn apply(&self, conn: &Connection, change: &EnvChange) -> Result<()> {
        let prior = self.check(change)?;
        let hive = Hive::CurrentUser;
        let name = prior.as_ref().map_or(change.name.clone(), |raw| raw.name.clone());
        let applied = change.new.as_ref().map(|new| {
            let kind = if new.expand { REG_TYPE_EXPAND_SZ } else { REG_TYPE_SZ };
            RegValue::string(name.as_str(), kind, &new.value)
        });
        let applied_record = applied
            .as_ref()
            .map_or((APPLIED_ABSENT_KIND, Vec::new()), |value| {
                (value.kind, value.bytes.clone())
            });
        let tweak_id = tweak_id_of(&name);

        // The record goes in before the write, so a crash in between still
        // leaves the prior value on file.
        conn.execute(
            "INSERT INTO tweak_undo (tweak_id, hive, subkey, value_name, prior_kind,
                                     prior_bytes, applied_kind, applied_bytes, applied_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                tweak_id,
                hive.short_name(),
                USER_KEY,
                name,
                prior.as_ref().map(|value| value.kind),
                prior.as_ref().map(|value| value.bytes.clone()),
                applied_record.0,
                applied_record.1,
                Utc::now().to_rfc3339(),
            ],
        )?;
        let row = conn.last_insert_rowid();
        // A write that never landed has nothing to undo, so its record goes.
        let forget = || conn.execute("DELETE FROM tweak_undo WHERE id = ?1", params![row]);

        let written = match &applied {
            Some(value) => self.registry.set_value(hive, USER_KEY, value),
            None => self.registry.delete_value(hive, USER_KEY, &name),
        };
        if let Err(err) = written {
            forget()?;
            return Err(err);
        }

        let after = self.registry.value(hive, USER_KEY, &name)?;
        if !same_value(after.as_ref(), &applied_record) {
            self.put_back(&name, prior.as_ref())?;
            forget()?;
            return Err(CoreError::Tweak(format!(
                "`{name}` did not read back as written; the old value was put back"
            )));
        }

        conn.execute(
            "UPDATE tweak_undo SET closed_at = ?2, close_reason = 'superseded'
             WHERE tweak_id = ?1 AND closed_at IS NULL AND id <> ?3",
            params![tweak_id, Utc::now().to_rfc3339(), row],
        )?;
        broadcast_environment_change();
        Ok(())
    }

    /// Put back what the last change of `name` replaced.
    ///
    /// # Errors
    /// Nothing to undo, a record outside `HKCU`, the value changed since
    /// WinKnife wrote it, or a read-back mismatch.
    pub fn undo(&self, conn: &Connection, name: &str) -> Result<()> {
        let row = open_row(conn, &tweak_id_of(name))?
            .ok_or_else(|| CoreError::Tweak(format!("nothing to undo for `{name}`")))?;
        if row.hive != Hive::CurrentUser.short_name() {
            return Err(CoreError::Tweak(format!(
                "undo record for `{name}` points outside HKCU"
            )));
        }
        let hive = Hive::CurrentUser;
        let live = self.registry.value(hive, USER_KEY, &row.value_name)?;
        if !same_value(live.as_ref(), &row.applied) {
            return Err(CoreError::Tweak(format!(
                "`{}` changed since WinKnife set it; left as it is",
                row.value_name
            )));
        }

        let prior = row.prior.as_ref().map(|(kind, bytes)| RegValue {
            name: row.value_name.clone(),
            kind: *kind,
            bytes: bytes.clone(),
        });
        self.put_back(&row.value_name, prior.as_ref())?;

        let after = self.registry.value(hive, USER_KEY, &row.value_name)?;
        let restored = match &row.prior {
            Some(prior) => same_value(after.as_ref(), prior),
            None => after.is_none(),
        };
        if !restored {
            return Err(CoreError::Tweak(format!(
                "`{}` did not read back as it was before",
                row.value_name
            )));
        }
        close_row(conn, row.id, "undone")?;
        broadcast_environment_change();
        Ok(())
    }

    fn put_back(&self, name: &str, prior: Option<&RegValue>) -> Result<()> {
        match prior {
            Some(prior) => self.registry.set_value(Hive::CurrentUser, USER_KEY, prior),
            None => self.registry.delete_value(Hive::CurrentUser, USER_KEY, name),
        }
    }
}

/// Registry entry. No scan, no findings: an editor page.
#[derive(Debug, Default)]
pub struct EnvVarsModule;

impl ToolModule for EnvVarsModule {
    fn id(&self) -> &'static str {
        "env-vars"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Environment Variables".to_owned(),
            description:
                "View user and system environment variables; edit user ones with a diff and undo."
                    .to_owned(),
            icon: "env".to_owned(),
            capabilities: ModuleCapabilities {
                has_scan: false,
                has_execute: false,
                has_settings: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::open_in_memory;
    use crate::core::platform::{MapEnv, MapRegistry};

    const PATH: &str = r"C:\Tools;%TOOLS%\bin;C:\Gone;c:\tools\;;%NOPE%\x";

    fn no_env() -> MapEnv {
        MapEnv::new()
    }

    fn service(registry: MapRegistry) -> (Arc<MapRegistry>, EnvVarService) {
        let registry = Arc::new(registry);
        (registry.clone(), EnvVarService::new(registry))
    }

    fn user(values: Vec<RegValue>) -> MapRegistry {
        MapRegistry::new().with_key(Hive::CurrentUser, USER_KEY, values)
    }

    fn val(value: &str, expand: bool) -> EnvValue {
        EnvValue {
            value: value.to_owned(),
            expand,
        }
    }

    fn change(name: &str, new: Option<EnvValue>, expected: Option<EnvValue>) -> EnvChange {
        EnvChange {
            name: name.to_owned(),
            scope: Scope::User,
            new,
            expected,
        }
    }

    fn live(registry: &MapRegistry, name: &str) -> Option<RegValue> {
        registry.value(Hive::CurrentUser, USER_KEY, name).unwrap()
    }

    fn kinds(entry: &PathEntry) -> Vec<FlagKind> {
        entry.flags.iter().map(|flag| flag.kind.clone()).collect()
    }

    #[test]
    fn split_and_join_round_trip() {
        for text in [PATH, "", ";", "a;", ";a;;b;", "single"] {
            assert_eq!(join_path(&split_path(text)), text);
        }
        assert_eq!(split_path("a;;b").len(), 3);
    }

    #[test]
    fn entries_flag_duplicates_missing_empty_and_unresolved() {
        let env = MapEnv::new().with("TOOLS", r"C:\Tools");
        let exists = |path: &Path| path == Path::new(r"C:\Tools") || path == Path::new(r"C:\Tools\bin");
        let entries = entries_of(PATH, true, &env, &exists);
        assert_eq!(entries.len(), 6);
        assert!(kinds(&entries[0]).is_empty());
        assert_eq!(entries[1].expanded.as_deref(), Some(r"C:\Tools\bin"));
        assert!(kinds(&entries[1]).is_empty());
        assert_eq!(kinds(&entries[2]), vec![FlagKind::Missing]);
        // `c:\tools\` is `C:\Tools` again: case and trailing `\` do not count.
        assert_eq!(kinds(&entries[3]), vec![FlagKind::Duplicate]);
        assert!(entries[3].flags[0].reason.contains("entry 1"));
        assert_eq!(kinds(&entries[4]), vec![FlagKind::Empty]);
        assert_eq!(kinds(&entries[5]), vec![FlagKind::Unresolved]);
        assert!(entries.iter().flat_map(|e| &e.flags).all(|f| !f.reason.is_empty()));
    }

    #[test]
    fn non_path_entries_are_not_checked_on_disk() {
        let entries = entries_of(".COM;.EXE;.BAT", false, &no_env(), &|_: &Path| false);
        assert!(entries.iter().all(|entry| entry.flags.is_empty()));
    }

    #[test]
    fn reg_sz_entries_are_not_expanded() {
        let env = MapEnv::new().with("TOOLS", r"C:\Tools");
        let entries = entries_of(r"%TOOLS%;x", false, &env, &|_: &Path| false);
        assert_eq!(entries[0].expanded, None);
        assert_eq!(kinds(&entries[0]), vec![FlagKind::Missing]);
    }

    #[test]
    fn validation_refuses_bad_names_and_values() {
        assert!(validate("GOOD_NAME", Some(&val("x", false))).is_ok());
        assert!(validate("", None).is_err());
        assert!(validate("   ", None).is_err());
        assert!(validate("A=B", None).is_err());
        assert!(validate("A\0B", None).is_err());
        assert!(validate(&"N".repeat(MAX_NAME_CHARS + 1), None).is_err());
        assert!(validate(&"N".repeat(MAX_NAME_CHARS), None).is_ok());
        assert!(validate("A", Some(&val(&"v".repeat(MAX_VALUE_CHARS + 1), false))).is_err());
        assert!(validate("A", Some(&val(&"v".repeat(MAX_VALUE_CHARS), false))).is_ok());
        assert!(validate("A", Some(&val("a\0b", false))).is_err());
    }

    #[test]
    fn create_then_undo_leaves_it_absent() {
        let conn = open_in_memory().unwrap();
        let (registry, service) = service(user(vec![]));
        service.apply(&conn, &change("NEWVAR", Some(val("hello", false)), None)).unwrap();
        let written = live(&registry, "NEWVAR").unwrap();
        assert_eq!(written.kind, REG_TYPE_SZ);
        assert_eq!(written.as_string().as_deref(), Some("hello"));

        service.undo(&conn, "NEWVAR").unwrap();
        assert!(live(&registry, "NEWVAR").is_none());
        assert!(service.undo(&conn, "NEWVAR").is_err(), "nothing left to undo");
    }

    #[test]
    fn modify_then_undo_restores_bytes_and_expand_kind() {
        let conn = open_in_memory().unwrap();
        let original = RegValue::string("Path", REG_TYPE_EXPAND_SZ, r"%USERPROFILE%\bin;C:\a");
        let (registry, service) = service(user(vec![original.clone()]));
        let shown = val(r"%USERPROFILE%\bin;C:\a", true);
        service
            .apply(&conn, &change("PATH", Some(val(r"C:\a;%USERPROFILE%\bin", true)), Some(shown)))
            .unwrap();
        let written = live(&registry, "Path").unwrap();
        assert_eq!(written.name, "Path", "existing casing kept");
        assert_eq!(written.kind, REG_TYPE_EXPAND_SZ);

        service.undo(&conn, "path").unwrap();
        assert_eq!(live(&registry, "Path"), Some(original));
    }

    #[test]
    fn kind_change_is_written_and_undone() {
        let conn = open_in_memory().unwrap();
        let original = RegValue::string("X", REG_TYPE_SZ, "%A%");
        let (registry, service) = service(user(vec![original.clone()]));
        service
            .apply(&conn, &change("X", Some(val("%A%", true)), Some(val("%A%", false))))
            .unwrap();
        assert_eq!(live(&registry, "X").unwrap().kind, REG_TYPE_EXPAND_SZ);
        service.undo(&conn, "X").unwrap();
        assert_eq!(live(&registry, "X"), Some(original));
    }

    #[test]
    fn delete_then_undo_brings_value_and_kind_back() {
        let conn = open_in_memory().unwrap();
        let original = RegValue::string("GOPATH", REG_TYPE_EXPAND_SZ, r"%USERPROFILE%\go");
        let (registry, service) = service(user(vec![original.clone()]));
        service
            .apply(&conn, &change("GOPATH", None, Some(val(r"%USERPROFILE%\go", true))))
            .unwrap();
        assert!(live(&registry, "GOPATH").is_none());
        let listing = service.list(&conn, &no_env(), &|_: &Path| true).unwrap();
        assert_eq!(listing.undo.len(), 1);
        assert!(listing.undo[0].deleted);

        service.undo(&conn, "GOPATH").unwrap();
        assert_eq!(live(&registry, "GOPATH"), Some(original));
        assert!(service.list(&conn, &no_env(), &|_: &Path| true).unwrap().undo.is_empty());
    }

    #[test]
    fn undo_of_delete_refuses_when_recreated_meanwhile() {
        let conn = open_in_memory().unwrap();
        let (registry, service) = service(user(vec![RegValue::string("A", REG_TYPE_SZ, "1")]));
        service.apply(&conn, &change("A", None, Some(val("1", false)))).unwrap();
        registry
            .set_value(Hive::CurrentUser, USER_KEY, &RegValue::string("A", REG_TYPE_SZ, "2"))
            .unwrap();
        assert!(service.undo(&conn, "A").is_err());
        assert_eq!(live(&registry, "A").unwrap().as_string().as_deref(), Some("2"));
    }

    #[test]
    fn undo_refuses_after_outside_change() {
        let conn = open_in_memory().unwrap();
        let (registry, service) = service(user(vec![RegValue::string("A", REG_TYPE_SZ, "1")]));
        service
            .apply(&conn, &change("A", Some(val("2", false)), Some(val("1", false))))
            .unwrap();
        registry
            .set_value(Hive::CurrentUser, USER_KEY, &RegValue::string("A", REG_TYPE_SZ, "3"))
            .unwrap();
        assert!(service.undo(&conn, "A").is_err());
        assert_eq!(live(&registry, "A").unwrap().as_string().as_deref(), Some("3"));
    }

    #[test]
    fn second_change_supersedes_first_undo_is_one_step() {
        let conn = open_in_memory().unwrap();
        let (registry, service) = service(user(vec![RegValue::string("A", REG_TYPE_SZ, "1")]));
        service.apply(&conn, &change("A", Some(val("2", false)), Some(val("1", false)))).unwrap();
        service.apply(&conn, &change("A", Some(val("3", false)), Some(val("2", false)))).unwrap();
        service.undo(&conn, "A").unwrap();
        assert_eq!(live(&registry, "A").unwrap().as_string().as_deref(), Some("2"));
        assert!(service.undo(&conn, "A").is_err());
    }

    #[test]
    fn system_scope_is_refused_and_hklm_untouched() {
        let conn = open_in_memory().unwrap();
        let registry = MapRegistry::new().with_key(
            Hive::LocalMachine,
            SYSTEM_KEY,
            vec![RegValue::string("Path", REG_TYPE_EXPAND_SZ, r"C:\Windows")],
        );
        let (registry, service) = service(registry);
        let mut system = change("Path", Some(val("x", false)), Some(val(r"C:\Windows", true)));
        system.scope = Scope::System;
        assert!(service.preview(&system).is_err());
        assert!(service.apply(&conn, &system).is_err());
        system.new = None;
        assert!(service.apply(&conn, &system).is_err());
        assert!(registry.writes().is_empty());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM tweak_undo", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn stale_expected_and_existing_add_are_refused() {
        let conn = open_in_memory().unwrap();
        let (registry, service) = service(user(vec![RegValue::string("A", REG_TYPE_SZ, "1")]));
        let stale = change("A", Some(val("2", false)), Some(val("0", false)));
        assert!(service.apply(&conn, &stale).is_err());
        let add = change("A", Some(val("2", false)), None);
        assert!(service.apply(&conn, &add).unwrap_err().to_string().contains("already exists"));
        let same = change("A", Some(val("1", false)), Some(val("1", false)));
        assert!(service.apply(&conn, &same).is_err());
        assert!(registry.writes().is_empty());
    }

    #[test]
    fn listing_marks_system_read_only_and_splits_lists() {
        let conn = open_in_memory().unwrap();
        let registry = user(vec![
            RegValue::string("Path", REG_TYPE_SZ, r"C:\a;C:\a"),
            RegValue::dword("Odd", 1),
        ])
        .with_key(
            Hive::LocalMachine,
            SYSTEM_KEY,
            vec![RegValue::string("OS", REG_TYPE_SZ, "Windows_NT")],
        );
        let (_, service) = service(registry);
        let listing = service.list(&conn, &no_env(), &|_: &Path| true).unwrap();
        let path = listing.user.iter().find(|v| v.name == "Path").unwrap();
        assert!(path.is_list && path.editable);
        assert_eq!(kinds(&path.entries[1]), vec![FlagKind::Duplicate]);
        let odd = listing.user.iter().find(|v| v.name == "Odd").unwrap();
        assert!(!odd.editable && odd.read_only_reason.is_some());
        assert!(listing.system.iter().all(|v| !v.editable && v.read_only_reason.is_some()));
    }

    #[test]
    fn preview_diffs_per_entry() {
        let (_, service) = service(user(vec![RegValue::string("P", REG_TYPE_SZ, "a;b")]));
        let preview = service
            .preview(&change("P", Some(val("a;c", false)), Some(val("a;b", false))))
            .unwrap();
        assert!(preview.diff.contains(&DiffLine::Removed { text: "b".to_owned() }));
        assert!(preview.diff.contains(&DiffLine::Added { text: "c".to_owned() }));
    }

    #[test]
    fn failed_write_leaves_no_record() {
        let conn = open_in_memory().unwrap();
        let (_, service) = service(
            user(vec![RegValue::string("A", REG_TYPE_SZ, "1")]).refusing_writes("blocked"),
        );
        assert!(service
            .apply(&conn, &change("A", Some(val("2", false)), Some(val("1", false))))
            .is_err());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM tweak_undo", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0, "a write that never landed leaves no record");
    }
}
