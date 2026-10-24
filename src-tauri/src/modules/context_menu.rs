//! Context-menu cleaner (C2): list what apps added to the Explorer right-click
//! menu, name the owner, and disable an entry reversibly.
//!
//! The scan only reads. Disabling writes one `HKCU` value and records the old
//! one in `tweak_undo` first (K-018), so Undo puts back exactly what was there.
//! Machine-wide (`HKLM`) entries are shown but never written: changing them
//! needs elevation, and this module does not ask for it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::core::error::{CoreError, Result};
use crate::core::executor::TweakControl;
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::platform::{
    Hive, RegValue, Registry, Signature, SignatureProbe, REG_TYPE_SZ,
};
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket, MAX_CONFIDENCE,
};

/// Prefix that marks a tweak id as belonging to this module (see [`TweakRouter`]).
pub const TWEAK_PREFIX: &str = "context-menu:";

/// The class keys whose menus show up on files, folders and drives.
const SCOPES: &[&str] = &[
    "*",
    "AllFilesystemObjects",
    "Directory",
    "Directory\\Background",
    "Drive",
    "Folder",
];

const BLOCKED: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Blocked";
/// Explorer skips a verb that carries this value, whatever its data.
const LEGACY_DISABLE: &str = "LegacyDisable";
const NEEDS_ADMIN: &str = "Machine-wide entry: needs admin; not changed.";

/// Observed value handed through the justification: the entry was enabled.
const OBSERVED_ENABLED: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    /// A `shell\<verb>` command.
    Verb,
    /// A `shellex\ContextMenuHandlers` COM extension.
    Handler,
}

/// One menu entry as the registry describes it, before any signature check.
#[derive(Debug, Clone)]
struct RawEntry {
    id: String,
    kind: EntryKind,
    hive: Hive,
    scope: String,
    name: String,
    label: Option<String>,
    /// The verb's command line, or the handler's CLSID.
    target: Option<String>,
    /// What the owner check looks at: the command line, or the handler DLL.
    file_hint: Option<String>,
    disabled: bool,
}

/// One menu entry as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextMenuEntry {
    pub id: String,
    pub kind: EntryKind,
    pub hive: String,
    pub scope: String,
    pub name: String,
    pub label: Option<String>,
    pub target: Option<String>,
    /// Registry key the entry lives under, for display.
    pub path: String,
    pub file: Option<String>,
    /// Signer of `file`; `None` means the owner is unknown.
    pub publisher: Option<String>,
    pub disabled: bool,
    /// `false` for machine-wide entries, which are never written.
    pub editable: bool,
    /// An open undo record exists for this entry.
    pub undoable: bool,
    pub reason: String,
}

/// The owner of an entry, as far as its file's signature shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Owner {
    Known { publisher: String },
    Unknown { why: String },
}

fn classify_owner(file: Option<&Path>, signature: Option<&Signature>) -> Owner {
    let Some(file) = file else {
        return Owner::Unknown {
            why: "the file it runs could not be found".to_owned(),
        };
    };
    match signature {
        Some(Signature::Signed {
            publisher: Some(publisher),
        }) if !publisher.trim().is_empty() => Owner::Known {
            publisher: publisher.clone(),
        },
        Some(Signature::Signed { .. }) => Owner::Unknown {
            why: format!("{} is signed but names no publisher", file.display()),
        },
        Some(Signature::Unsigned) => Owner::Unknown {
            why: format!("{} is not signed", file.display()),
        },
        Some(Signature::Unknown { reason }) => Owner::Unknown {
            why: format!("the signature of {} could not be checked: {reason}", file.display()),
        },
        None => Owner::Unknown {
            why: format!("the signature of {} was not checked", file.display()),
        },
    }
}

fn file_name(file: &Path) -> String {
    file.file_name()
        .map_or_else(|| file.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn entry_reason(owner: &Owner, file: Option<&Path>, editable: bool) -> String {
    let who = match (owner, file) {
        (Owner::Known { publisher }, Some(file)) => {
            format!("Added by {publisher} ({}).", file_name(file))
        }
        (Owner::Known { publisher }, None) => format!("Added by {publisher}."),
        (Owner::Unknown { why }, _) => format!("Owner unknown: {why}. Needs review."),
    };
    if editable {
        format!("{who} Disabling hides it from the right-click menu; Undo brings it back.")
    } else {
        format!("{who} {NEEDS_ADMIN}")
    }
}

fn classes(scope: &str) -> String {
    format!("Software\\Classes\\{scope}")
}

fn entry_key(entry: &RawEntry) -> String {
    match entry.kind {
        EntryKind::Verb => format!("{}\\shell\\{}", classes(&entry.scope), entry.name),
        EntryKind::Handler => format!(
            "{}\\shellex\\ContextMenuHandlers\\{}",
            classes(&entry.scope),
            entry.name
        ),
    }
}

fn same_value(value: Option<&RegValue>, expected: &(u32, Vec<u8>)) -> bool {
    value.is_some_and(|value| value.kind == expected.0 && value.bytes == expected.1)
}

struct UndoRow {
    id: i64,
    hive: String,
    subkey: String,
    value_name: String,
    prior: Option<(u32, Vec<u8>)>,
    applied: (u32, Vec<u8>),
}

fn open_row(conn: &Connection, tweak_id: &str) -> Result<Option<UndoRow>> {
    Ok(conn
        .query_row(
            "SELECT id, hive, subkey, value_name, prior_kind, prior_bytes,
                    applied_kind, applied_bytes
             FROM tweak_undo
             WHERE tweak_id = ?1 AND closed_at IS NULL
             ORDER BY id DESC LIMIT 1",
            params![tweak_id],
            |row| {
                let prior_kind: Option<u32> = row.get(4)?;
                let prior_bytes: Option<Vec<u8>> = row.get(5)?;
                Ok(UndoRow {
                    id: row.get(0)?,
                    hive: row.get(1)?,
                    subkey: row.get(2)?,
                    value_name: row.get(3)?,
                    prior: prior_kind.zip(prior_bytes),
                    applied: (row.get(6)?, row.get(7)?),
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

/// Turns a command line or DLL value into the file it names.
pub type Resolver = fn(&str) -> Option<PathBuf>;

/// Production resolver: a command line or path on disk, else a bare DLL name
/// in `System32` (how many shell handlers register themselves).
fn resolve_on_disk(target: &str) -> Option<PathBuf> {
    if let Some(found) = crate::modules::startup::executable_of(target) {
        return Some(found);
    }
    let bare = target.trim().trim_matches('"');
    if bare.is_empty() || bare.contains('\\') || bare.contains('/') {
        return None;
    }
    let root = std::env::var("SystemRoot").ok()?;
    let candidate = PathBuf::from(root).join("System32").join(bare);
    candidate.is_file().then_some(candidate)
}

/// The scan, the owner check and the reversible disable.
pub struct ContextMenuService {
    registry: Arc<dyn Registry>,
    signatures: Arc<dyn SignatureProbe>,
    resolve: Resolver,
}

impl ContextMenuService {
    pub fn new(registry: Arc<dyn Registry>, signatures: Arc<dyn SignatureProbe>) -> Self {
        Self::with_resolver(registry, signatures, resolve_on_disk)
    }

    pub fn with_resolver(
        registry: Arc<dyn Registry>,
        signatures: Arc<dyn SignatureProbe>,
        resolve: Resolver,
    ) -> Self {
        Self {
            registry,
            signatures,
            resolve,
        }
    }

    /// Read-only: every entry under every scope in both hives, with its owner.
    ///
    /// # Errors
    /// Returns an error only when the undo table cannot be read.
    pub fn scan(&self, conn: &Connection) -> Result<Vec<ContextMenuEntry>> {
        self.collect()
            .into_iter()
            .map(|raw| self.describe(conn, raw))
            .collect()
    }

    fn describe(&self, conn: &Connection, raw: RawEntry) -> Result<ContextMenuEntry> {
        let file = raw.file_hint.as_deref().and_then(self.resolve);
        let signature = file.as_deref().map(|file| self.signatures.check(file));
        let owner = classify_owner(file.as_deref(), signature.as_ref());
        let editable = raw.hive == Hive::CurrentUser;
        let undoable = open_row(conn, &tweak_id_of(&raw.id))?.is_some();
        Ok(ContextMenuEntry {
            reason: entry_reason(&owner, file.as_deref(), editable),
            path: format!("{}\\{}", raw.hive.short_name(), entry_key(&raw)),
            publisher: match owner {
                Owner::Known { publisher } => Some(publisher),
                Owner::Unknown { .. } => None,
            },
            file: file.map(|file| file.display().to_string()),
            hive: raw.hive.short_name().to_owned(),
            id: raw.id,
            kind: raw.kind,
            scope: raw.scope,
            name: raw.name,
            label: raw.label,
            target: raw.target,
            disabled: raw.disabled,
            editable,
            undoable,
        })
    }

    fn collect(&self) -> Vec<RawEntry> {
        let blocked = self.blocked_clsids();
        let mut out = Vec::new();
        for hive in [Hive::CurrentUser, Hive::LocalMachine] {
            for scope in SCOPES {
                out.extend(self.verbs(hive, scope));
                out.extend(self.handlers(hive, scope, &blocked));
            }
        }
        out
    }

    fn subkeys(&self, hive: Hive, key: &str) -> Vec<String> {
        self.registry.subkeys(hive, key).unwrap_or_else(|err| {
            tracing::warn!(hive = hive.short_name(), key, error = %err, "could not list a registry key");
            Vec::new()
        })
    }

    fn string(&self, hive: Hive, key: &str, name: &str) -> Option<String> {
        match self.registry.value(hive, key, name) {
            Ok(value) => value
                .and_then(|value| value.as_string())
                .filter(|text| !text.trim().is_empty()),
            Err(err) => {
                tracing::warn!(hive = hive.short_name(), key, error = %err, "could not read a registry value");
                None
            }
        }
    }

    fn has_value(&self, hive: Hive, key: &str, name: &str) -> bool {
        matches!(self.registry.value(hive, key, name), Ok(Some(_)))
    }

    fn verbs(&self, hive: Hive, scope: &str) -> Vec<RawEntry> {
        let shell = format!("{}\\shell", classes(scope));
        self.subkeys(hive, &shell)
            .into_iter()
            .map(|name| {
                let key = format!("{shell}\\{name}");
                let command = self.string(hive, &format!("{key}\\command"), "");
                RawEntry {
                    id: format!("{}|{scope}|shell|{name}", hive.short_name()),
                    kind: EntryKind::Verb,
                    hive,
                    scope: scope.to_owned(),
                    label: self
                        .string(hive, &key, "MUIVerb")
                        .or_else(|| self.string(hive, &key, "")),
                    target: command.clone(),
                    file_hint: command,
                    disabled: self.has_value(hive, &key, LEGACY_DISABLE),
                    name,
                }
            })
            .collect()
    }

    fn handlers(&self, hive: Hive, scope: &str, blocked: &[String]) -> Vec<RawEntry> {
        let handlers = format!("{}\\shellex\\ContextMenuHandlers", classes(scope));
        self.subkeys(hive, &handlers)
            .into_iter()
            .map(|name| {
                let clsid = self
                    .string(hive, &format!("{handlers}\\{name}"), "")
                    .map(|text| text.trim().to_owned())
                    .filter(|text| text.starts_with('{'))
                    .or_else(|| name.starts_with('{').then(|| name.clone()));
                let (label, dll) = match &clsid {
                    Some(clsid) => (self.clsid_value(clsid, ""), self.clsid_value(&format!("{clsid}\\InprocServer32"), "")),
                    None => (None, None),
                };
                RawEntry {
                    id: format!("{}|{scope}|shellex|{name}", hive.short_name()),
                    kind: EntryKind::Handler,
                    hive,
                    scope: scope.to_owned(),
                    label,
                    disabled: clsid.as_ref().is_some_and(|clsid| {
                        blocked.iter().any(|entry| entry.eq_ignore_ascii_case(clsid))
                    }),
                    target: clsid,
                    file_hint: dll,
                    name,
                }
            })
            .collect()
    }

    /// A value under `Software\Classes\CLSID\<rest>`, per-user first.
    fn clsid_value(&self, rest: &str, name: &str) -> Option<String> {
        let key = format!("Software\\Classes\\CLSID\\{rest}");
        self.string(Hive::CurrentUser, &key, name)
            .or_else(|| self.string(Hive::LocalMachine, &key, name))
    }

    fn blocked_clsids(&self) -> Vec<String> {
        [Hive::CurrentUser, Hive::LocalMachine]
            .into_iter()
            .flat_map(|hive| self.registry.values(hive, BLOCKED).unwrap_or_default())
            .map(|value| value.name)
            .collect()
    }

    fn find(&self, entry_id: &str) -> Result<RawEntry> {
        self.collect()
            .into_iter()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| CoreError::Tweak(format!("context-menu entry `{entry_id}` is gone")))
    }

    /// The single `HKCU` value that disables `entry`, or why there is none.
    fn disable_target(&self, entry: &RawEntry) -> std::result::Result<(String, String), String> {
        if entry.hive != Hive::CurrentUser {
            return Err(NEEDS_ADMIN.to_owned());
        }
        if entry.disabled {
            return Err(format!("`{}` is already disabled", entry.name));
        }
        match entry.kind {
            EntryKind::Verb => Ok((entry_key(entry), LEGACY_DISABLE.to_owned())),
            EntryKind::Handler => {
                let clsid = entry
                    .target
                    .clone()
                    .ok_or_else(|| format!("`{}` names no CLSID; not changed", entry.name))?;
                // Blocking a CLSID blocks it everywhere, so a CLSID a
                // machine-wide handler also uses is left to the admin.
                let shared = self.collect().iter().any(|other| {
                    other.hive == Hive::LocalMachine
                        && other.kind == EntryKind::Handler
                        && other
                            .target
                            .as_deref()
                            .is_some_and(|target| target.eq_ignore_ascii_case(&clsid))
                });
                if shared {
                    return Err(format!(
                        "`{}` shares its CLSID with a machine-wide entry: needs admin; not changed",
                        entry.name
                    ));
                }
                Ok((BLOCKED.to_owned(), clsid))
            }
        }
    }

    /// Put back what the last disable of `entry_id` replaced.
    ///
    /// # Errors
    /// Fails when there is nothing to undo, the record points outside `HKCU`,
    /// or the value changed since WinKnife wrote it.
    pub fn undo(&self, conn: &Connection, entry_id: &str) -> Result<()> {
        let tweak_id = tweak_id_of(entry_id);
        let row = open_row(conn, &tweak_id)?
            .ok_or_else(|| CoreError::Tweak(format!("nothing to undo for `{entry_id}`")))?;
        if row.hive != Hive::CurrentUser.short_name() {
            return Err(CoreError::Tweak(format!(
                "undo record for `{entry_id}` points outside HKCU"
            )));
        }
        let hive = Hive::CurrentUser;
        let live = self.registry.value(hive, &row.subkey, &row.value_name)?;
        if !same_value(live.as_ref(), &row.applied) {
            return Err(CoreError::Tweak(format!(
                "`{}` changed since WinKnife set it; left as it is",
                row.value_name
            )));
        }

        match &row.prior {
            Some((kind, bytes)) => self.registry.set_value(
                hive,
                &row.subkey,
                &RegValue {
                    name: row.value_name.clone(),
                    kind: *kind,
                    bytes: bytes.clone(),
                },
            )?,
            None => self
                .registry
                .delete_value(hive, &row.subkey, &row.value_name)?,
        }

        let after = self.registry.value(hive, &row.subkey, &row.value_name)?;
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
        close_row(conn, row.id, "undone")
    }

    fn roll_back(&self, subkey: &str, name: &str, prior: Option<&RegValue>) -> Result<()> {
        match prior {
            Some(prior) => self.registry.set_value(Hive::CurrentUser, subkey, prior),
            None => self.registry.delete_value(Hive::CurrentUser, subkey, name),
        }
    }
}

fn tweak_id_of(entry_id: &str) -> String {
    format!("{TWEAK_PREFIX}{entry_id}")
}

impl TweakControl for ContextMenuService {
    fn refusal(&self, tweak_id: &str, observed: Option<u32>) -> Option<String> {
        let entry_id = tweak_id.strip_prefix(TWEAK_PREFIX).unwrap_or(tweak_id);
        let entry = match self.find(entry_id) {
            Ok(entry) => entry,
            Err(err) => return Some(format!("skipped: {err}")),
        };
        if observed != Some(OBSERVED_ENABLED) {
            return Some("skipped: this entry was not enabled when scanned".to_owned());
        }
        self.disable_target(&entry)
            .err()
            .map(|reason| format!("skipped: {reason}"))
    }

    fn apply(&self, conn: &Connection, tweak_id: &str) -> Result<()> {
        let entry_id = tweak_id.strip_prefix(TWEAK_PREFIX).unwrap_or(tweak_id);
        let entry = self.find(entry_id)?;
        let (subkey, value_name) = self.disable_target(&entry).map_err(CoreError::Tweak)?;
        let hive = Hive::CurrentUser;
        let prior = self.registry.value(hive, &subkey, &value_name)?;
        let applied = RegValue::string(value_name.as_str(), REG_TYPE_SZ, "");
        let tweak_id = tweak_id_of(&entry.id);

        // The record goes in before the write, so a crash in between still
        // leaves the prior value on file.
        conn.execute(
            "INSERT INTO tweak_undo (tweak_id, hive, subkey, value_name, prior_kind,
                                     prior_bytes, applied_kind, applied_bytes, applied_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                tweak_id,
                hive.short_name(),
                subkey,
                value_name,
                prior.as_ref().map(|value| value.kind),
                prior.as_ref().map(|value| value.bytes.clone()),
                applied.kind,
                applied.bytes,
                Utc::now().to_rfc3339(),
            ],
        )?;
        let row = conn.last_insert_rowid();
        // A write that never landed has nothing to undo, so its record goes.
        let forget = || conn.execute("DELETE FROM tweak_undo WHERE id = ?1", params![row]);

        if let Err(err) = self.registry.set_value(hive, &subkey, &applied) {
            forget()?;
            return Err(err);
        }

        let after = self.registry.value(hive, &subkey, &value_name)?;
        if !same_value(after.as_ref(), &(applied.kind, applied.bytes.clone())) {
            self.roll_back(&subkey, &value_name, prior.as_ref())?;
            forget()?;
            return Err(CoreError::Tweak(format!(
                "`{value_name}` did not read back as written; the old value was put back"
            )));
        }

        conn.execute(
            "UPDATE tweak_undo SET closed_at = ?2, close_reason = 'superseded'
             WHERE tweak_id = ?1 AND closed_at IS NULL AND id <> ?3",
            params![tweak_id, Utc::now().to_rfc3339(), row],
        )?;
        Ok(())
    }
}

/// The finding and the reversible "disable" suggestion for one entry, or
/// `None` for an entry this module will not change (machine-wide, already
/// disabled).
pub fn recommendation(entry: &ContextMenuEntry) -> Option<(Finding, Recommendation)> {
    if !entry.editable || entry.disabled {
        return None;
    }
    let mut finding = Finding::new(
        "context-menu",
        PathBuf::from(&entry.path),
        FindingKind::RegistryEntry,
    );
    finding.meta = serde_json::json!({ "contextMenuId": entry.id });
    let recommendation = Recommendation::new(
        vec![finding.id],
        RiskBucket::NeedsReview,
        MAX_CONFIDENCE,
        Action::ApplyTweak,
        entry.reason.clone(),
        0,
        Justification::Tweak {
            tweak_id: tweak_id_of(&entry.id),
            observed: Some(OBSERVED_ENABLED),
        },
    )
    .map_err(|err| tracing::warn!(entry = entry.id.as_str(), error = %err, "context-menu recommendation rejected"))
    .ok()?;
    Some((finding, recommendation))
}

/// Sends each tweak id to its owner, so the executor keeps one `TweakControl`.
pub struct TweakRouter<'a> {
    pub tweaks: &'a dyn TweakControl,
    pub context_menu: &'a ContextMenuService,
}

impl TweakControl for TweakRouter<'_> {
    fn refusal(&self, tweak_id: &str, observed: Option<u32>) -> Option<String> {
        if tweak_id.starts_with(TWEAK_PREFIX) {
            self.context_menu.refusal(tweak_id, observed)
        } else {
            self.tweaks.refusal(tweak_id, observed)
        }
    }

    fn apply(&self, conn: &Connection, tweak_id: &str) -> Result<()> {
        if tweak_id.starts_with(TWEAK_PREFIX) {
            self.context_menu.apply(conn, tweak_id)
        } else {
            self.tweaks.apply(conn, tweak_id)
        }
    }
}

/// The context-menu cleaner (C2).
#[derive(Debug, Default)]
pub struct ContextMenuModule;

impl ToolModule for ContextMenuModule {
    fn id(&self) -> &'static str {
        "context-menu"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Context menu".to_owned(),
            description: "Lists what apps added to the right-click menu and disables an entry, reversibly."
                .to_owned(),
            icon: "menu".to_owned(),
            capabilities: ModuleCapabilities {
                has_scan: true,
                has_execute: true,
                has_settings: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::open_in_memory;
    use crate::core::platform::{MapRegistry, MapSignatures};

    const APP: &str = "C:\\Apps\\Tool\\tool.exe";
    const DLL: &str = "C:\\Apps\\Ext\\ext.dll";
    const CLSID: &str = "{11111111-2222-3333-4444-555555555555}";

    fn test_resolve(target: &str) -> Option<PathBuf> {
        let target = target.trim();
        let path = match target.strip_prefix('"') {
            Some(rest) => rest.split('"').next().unwrap_or(rest),
            None => target.split(' ').next().unwrap_or(target),
        };
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    fn signed(publisher: &str) -> Signature {
        Signature::Signed {
            publisher: Some(publisher.to_owned()),
        }
    }

    fn service(registry: MapRegistry, signatures: MapSignatures) -> (Arc<MapRegistry>, ContextMenuService) {
        let registry = Arc::new(registry);
        let service = ContextMenuService::with_resolver(
            registry.clone(),
            Arc::new(signatures),
            test_resolve,
        );
        (registry, service)
    }

    fn with_verb(registry: MapRegistry, hive: Hive, verb: &str, command: &str) -> MapRegistry {
        let key = format!("Software\\Classes\\*\\shell\\{verb}");
        registry
            .with_key(hive, &key, vec![RegValue::string("", REG_TYPE_SZ, "Open with Tool")])
            .with_key(
                hive,
                &format!("{key}\\command"),
                vec![RegValue::string("", REG_TYPE_SZ, command)],
            )
    }

    fn with_handler(registry: MapRegistry, hive: Hive, name: &str) -> MapRegistry {
        registry
            .with_key(
                hive,
                &format!("Software\\Classes\\Directory\\shellex\\ContextMenuHandlers\\{name}"),
                vec![RegValue::string("", REG_TYPE_SZ, CLSID)],
            )
            .with_key(
                Hive::LocalMachine,
                &format!("Software\\Classes\\CLSID\\{CLSID}"),
                vec![RegValue::string("", REG_TYPE_SZ, "Ext Shell Extension")],
            )
            .with_key(
                Hive::LocalMachine,
                &format!("Software\\Classes\\CLSID\\{CLSID}\\InprocServer32"),
                vec![RegValue::string("", REG_TYPE_SZ, DLL)],
            )
    }

    fn open_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM tweak_undo WHERE closed_at IS NULL", [], |row| row.get(0))
            .expect("count")
    }

    fn only(entries: &[ContextMenuEntry], kind: EntryKind) -> &ContextMenuEntry {
        let found: Vec<_> = entries.iter().filter(|entry| entry.kind == kind).collect();
        assert_eq!(found.len(), 1, "{entries:?}");
        found[0]
    }

    #[test]
    fn owner_is_known_only_for_a_named_signer() {
        let file = Path::new(DLL);
        assert_eq!(
            classify_owner(Some(file), Some(&signed("Contoso Ltd"))),
            Owner::Known {
                publisher: "Contoso Ltd".to_owned()
            }
        );
        for unknown in [
            classify_owner(None, None),
            classify_owner(Some(file), Some(&Signature::Unsigned)),
            classify_owner(Some(file), Some(&Signature::Signed { publisher: None })),
            classify_owner(Some(file), Some(&signed("  "))),
            classify_owner(
                Some(file),
                Some(&Signature::Unknown {
                    reason: "timeout".to_owned(),
                }),
            ),
        ] {
            assert!(matches!(unknown, Owner::Unknown { .. }), "{unknown:?}");
        }
    }

    #[test]
    fn scan_names_the_owner_and_flags_unknown_ones_for_review() {
        let registry = with_handler(
            with_verb(MapRegistry::new(), Hive::CurrentUser, "tool", &format!("\"{APP}\" \"%1\"")),
            Hive::CurrentUser,
            "Ext",
        );
        let signatures = MapSignatures::new()
            .with(APP, signed("Contoso Ltd"))
            .defaulting_to(Signature::Unsigned);
        let (_, service) = service(registry, signatures);
        let conn = open_in_memory().expect("db");
        let entries = service.scan(&conn).expect("scan");

        let verb = only(&entries, EntryKind::Verb);
        assert_eq!(verb.publisher.as_deref(), Some("Contoso Ltd"));
        assert!(verb.reason.starts_with("Added by Contoso Ltd (tool.exe)."), "{}", verb.reason);
        assert!(verb.editable && !verb.disabled);

        let handler = only(&entries, EntryKind::Handler);
        assert_eq!(handler.publisher, None);
        assert_eq!(handler.label.as_deref(), Some("Ext Shell Extension"));
        assert_eq!(handler.file.as_deref(), Some(DLL));
        assert!(handler.reason.contains("Needs review"), "{}", handler.reason);
    }

    #[test]
    fn verb_disable_then_undo_round_trips() {
        let (registry, service) = service(
            with_verb(MapRegistry::new(), Hive::CurrentUser, "tool", APP),
            MapSignatures::new().defaulting_to(signed("Contoso Ltd")),
        );
        let conn = open_in_memory().expect("db");
        let entry = service.scan(&conn).expect("scan").remove(0);
        let (_, rec) = recommendation(&entry).expect("recommended");
        let Justification::Tweak { tweak_id, observed } = &rec.justification else {
            panic!("not a tweak");
        };
        assert_eq!(rec.risk, RiskBucket::NeedsReview);
        assert_eq!(service.refusal(tweak_id, *observed), None);

        service.apply(&conn, tweak_id).expect("apply");
        let key = "Software\\Classes\\*\\shell\\tool";
        assert!(service.registry.value(Hive::CurrentUser, key, LEGACY_DISABLE).expect("read").is_some());
        let after = service.scan(&conn).expect("scan").remove(0);
        assert!(after.disabled && after.undoable);
        assert!(service.refusal(tweak_id, *observed).is_some(), "already disabled");

        service.undo(&conn, &entry.id).expect("undo");
        assert!(service.registry.value(Hive::CurrentUser, key, LEGACY_DISABLE).expect("read").is_none());
        assert_eq!(open_rows(&conn), 0);
        assert!(registry.writes().iter().all(|(hive, ..)| *hive == Hive::CurrentUser));
        assert!(!service.scan(&conn).expect("scan").remove(0).disabled);
    }

    #[test]
    fn handler_disable_blocks_the_clsid_per_user_and_undo_removes_it() {
        let (registry, service) = service(
            with_handler(MapRegistry::new(), Hive::CurrentUser, "Ext"),
            MapSignatures::new().defaulting_to(signed("Contoso Ltd")),
        );
        let conn = open_in_memory().expect("db");
        let entry = service.scan(&conn).expect("scan").remove(0);
        let tweak_id = tweak_id_of(&entry.id);

        service.apply(&conn, &tweak_id).expect("apply");
        assert_eq!(
            registry.snapshot(Hive::CurrentUser, BLOCKED),
            vec![RegValue::string(CLSID, REG_TYPE_SZ, "")]
        );
        assert!(service.scan(&conn).expect("scan").remove(0).disabled);

        service.undo(&conn, &entry.id).expect("undo");
        assert!(registry.snapshot(Hive::CurrentUser, BLOCKED).is_empty());
        assert!(registry.writes().iter().all(|(hive, ..)| *hive == Hive::CurrentUser));
    }

    #[test]
    fn machine_wide_entries_are_never_written() {
        let registry = with_handler(
            with_verb(MapRegistry::new(), Hive::LocalMachine, "tool", APP),
            Hive::LocalMachine,
            "Ext",
        );
        let (registry, service) = service(registry, MapSignatures::new().defaulting_to(signed("Contoso Ltd")));
        let conn = open_in_memory().expect("db");
        let entries = service.scan(&conn).expect("scan");
        assert_eq!(entries.len(), 2);
        for entry in &entries {
            assert!(!entry.editable);
            assert!(entry.reason.contains("needs admin; not changed"), "{}", entry.reason);
            assert!(recommendation(entry).is_none());
            let tweak_id = tweak_id_of(&entry.id);
            assert!(service.refusal(&tweak_id, Some(OBSERVED_ENABLED)).is_some());
            assert!(service.apply(&conn, &tweak_id).is_err());
        }
        assert!(registry.writes().is_empty(), "{:?}", registry.writes());
        assert_eq!(open_rows(&conn), 0);
    }

    #[test]
    fn a_user_handler_sharing_a_machine_clsid_is_left_alone() {
        let registry = with_handler(
            with_handler(MapRegistry::new(), Hive::CurrentUser, "Ext"),
            Hive::LocalMachine,
            "Ext",
        );
        let (registry, service) = service(registry, MapSignatures::new().defaulting_to(signed("Contoso Ltd")));
        let conn = open_in_memory().expect("db");
        let user = service
            .scan(&conn)
            .expect("scan")
            .into_iter()
            .find(|entry| entry.editable)
            .expect("user entry");
        assert!(service.apply(&conn, &tweak_id_of(&user.id)).is_err());
        assert!(registry.writes().is_empty());
    }

    #[test]
    fn undo_refuses_a_value_changed_since() {
        let (registry, service) = service(
            with_verb(MapRegistry::new(), Hive::CurrentUser, "tool", APP),
            MapSignatures::new().defaulting_to(signed("Contoso Ltd")),
        );
        let conn = open_in_memory().expect("db");
        let entry = service.scan(&conn).expect("scan").remove(0);
        service.apply(&conn, &tweak_id_of(&entry.id)).expect("apply");
        registry
            .set_value(
                Hive::CurrentUser,
                "Software\\Classes\\*\\shell\\tool",
                &RegValue::string(LEGACY_DISABLE, REG_TYPE_SZ, "someone else"),
            )
            .expect("write");
        assert!(service.undo(&conn, &entry.id).is_err());
        assert_eq!(open_rows(&conn), 1);
    }

    #[test]
    fn router_sends_prefixed_ids_to_the_context_menu_service() {
        struct Never;
        impl TweakControl for Never {
            fn refusal(&self, _: &str, _: Option<u32>) -> Option<String> {
                Some("tweaks".to_owned())
            }
            fn apply(&self, _: &Connection, _: &str) -> Result<()> {
                Err(CoreError::Tweak("tweaks".to_owned()))
            }
        }
        let (_, service) = service(MapRegistry::new(), MapSignatures::new());
        let router = TweakRouter {
            tweaks: &Never,
            context_menu: &service,
        };
        assert_eq!(router.refusal("show-file-extensions", None).as_deref(), Some("tweaks"));
        let routed = router.refusal("context-menu:HKCU|*|shell|x", Some(0)).expect("refused");
        assert!(routed.contains("gone"), "{routed}");
    }
}
