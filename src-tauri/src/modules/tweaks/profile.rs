//! Settings profiles (C10): the applied tweaks plus the app settings, as a
//! versioned JSON file. Import is a trust boundary — the file may come from
//! anywhere — so it is size-capped before parsing, unknown fields and wrong
//! types are refused, and every tweak id must be a known one inside the fence.
//! Importing never writes by itself: tweaks go through the ordinary scan +
//! execute path, settings through the ordinary settings save.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::TweakService;
use crate::core::error::{CoreError, Result};

/// The only schema version this build reads and writes.
pub const PROFILE_VERSION: u32 = 1;
/// Larger than any real profile by far; anything bigger is refused unread.
pub const MAX_PROFILE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    /// Catalogue tweak ids to apply.
    pub tweaks: Vec<String>,
    pub settings: ProfileSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileSettings {
    /// Cleaner pack id → enabled.
    pub cleaner_packs: BTreeMap<String, bool>,
    pub duplicates: ProfileDuplicates,
    pub quarantine: ProfileQuarantine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileDuplicates {
    pub min_size_bytes: u64,
    pub roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileQuarantine {
    pub retention_days: i64,
    pub max_bytes: u64,
}

fn invalid(reason: impl std::fmt::Display) -> CoreError {
    CoreError::Tweak(format!("this profile cannot be imported: {reason}"))
}

/// Parse and check a profile's bytes. Writes nothing.
///
/// # Errors
/// Refuses an oversized file, malformed JSON, unknown fields, wrong types, an
/// unsupported version, or a tweak id that is unknown or fenced.
pub fn parse(bytes: &[u8], service: &TweakService) -> Result<Profile> {
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(invalid(format!(
            "it is larger than {MAX_PROFILE_BYTES} bytes"
        )));
    }
    let profile: Profile = serde_json::from_slice(bytes).map_err(invalid)?;
    if profile.version != PROFILE_VERSION {
        return Err(invalid(format!(
            "it is version {}, and this WinKnife reads version {PROFILE_VERSION}",
            profile.version
        )));
    }
    for id in &profile.tweaks {
        service.find(id).map_err(invalid)?;
    }
    Ok(profile)
}

/// Read a profile file, refusing it unread past [`MAX_PROFILE_BYTES`].
///
/// # Errors
/// As [`parse`], plus any read error.
pub fn read(path: &Path, service: &TweakService) -> Result<Profile> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_PROFILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    parse(&bytes, service)
}

/// The profile as pretty JSON, ready to save.
///
/// # Errors
/// Only if serialisation fails.
pub fn to_json(profile: &Profile) -> Result<String> {
    serde_json::to_string_pretty(profile).map_err(|err| CoreError::Tweak(err.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::{Restart, Tweak, Undoability, CATALOG};
    use super::*;
    use crate::core::db::open_in_memory;
    use crate::core::platform::{Hive, MapRegistry};

    const VALID: &str = r#"{
        "version": 1,
        "tweaks": ["show-file-extensions"],
        "settings": {
            "cleanerPacks": {"browsers": true},
            "duplicates": {"minSizeBytes": 1048576, "roots": ["C:\\Users\\me\\Pictures"]},
            "quarantine": {"retentionDays": 30, "maxBytes": 1073741824}
        }
    }"#;

    fn fenced() -> Tweak {
        Tweak {
            id: "disable-updates",
            hive: Hive::CurrentUser,
            subkey: r"Software\Policies\Microsoft\Windows\WindowsUpdate\AU",
            value_name: "NoAutoUpdate",
            target: 1,
            restart: Restart::None,
            undo: Undoability::Reversible,
            ..CATALOG[0]
        }
    }

    fn service() -> (Arc<MapRegistry>, TweakService) {
        let registry = Arc::new(MapRegistry::new());
        let mut catalog = CATALOG.to_vec();
        catalog.push(fenced());
        let service = TweakService::with_catalog(registry.clone(), catalog);
        (registry, service)
    }

    fn with_tweak(id: &str) -> String {
        VALID.replace("show-file-extensions", id)
    }

    fn refused(json: &str) -> String {
        let (registry, service) = service();
        let conn = open_in_memory().expect("db");
        let err = parse(json.as_bytes(), &service).expect_err("refused");
        assert!(registry.writes().is_empty(), "nothing written");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM tweak_undo", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 0, "no undo rows");
        err.to_string()
    }

    #[test]
    fn a_valid_profile_round_trips() {
        let (_, service) = service();
        let profile = parse(VALID.as_bytes(), &service).expect("valid");
        let again = parse(to_json(&profile).expect("json").as_bytes(), &service).expect("again");
        assert_eq!(profile, again);
    }

    #[test]
    fn a_fenced_or_unknown_tweak_is_refused_without_writing() {
        assert!(refused(&with_tweak("disable-updates")).contains("cannot be imported"));
        assert!(refused(&with_tweak("format-c")).contains("unknown tweak"));
    }

    #[test]
    fn unknown_fields_bad_types_and_other_versions_are_refused() {
        refused(&VALID.replace(r#""version": 1,"#, r#""version": 1, "extra": true,"#));
        refused(&VALID.replace(r#""maxBytes": 1073741824"#, r#""maxBytes": 1, "purge": 1"#));
        refused(&VALID.replace(r#""retentionDays": 30"#, r#""retentionDays": "30""#));
        refused(&VALID.replace(r#""browsers": true"#, r#""browsers": "yes""#));
        refused(&VALID.replace(r#""version": 1"#, r#""version": 2"#));
        refused("[]");
        refused("not json");
    }

    #[test]
    fn an_oversized_profile_is_refused_before_parsing() {
        let padded = format!("{VALID}{}", " ".repeat(MAX_PROFILE_BYTES as usize));
        assert!(refused(&padded).contains("larger than"));
    }
}
