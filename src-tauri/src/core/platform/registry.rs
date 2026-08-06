//! Windows registry access, behind a trait so it can be faked in tests.
//!
//! # Why a wrapper
//!
//! The startup manager reads `Run`/`RunOnce` and writes the `StartupApproved`
//! flags. Both are real machine state: a unit test that reached the real
//! registry would enumerate whatever the developer happens to have installed
//! (so its assertions would be about their machine, not about the code) and a
//! write would change what runs at their next boot. Module spec 05 asks for a
//! mockable wrapper for exactly this reason.
//!
//! Two things enforce it rather than one:
//!
//! 1. Every caller holds an `Arc<dyn Registry>`, and the tests inject
//!    [`MapRegistry`].
//! 2. [`SystemRegistry`] refuses to **write** until
//!    [`crate::core::platform::real_os_effects_armed`] is true, which only
//!    `lib.rs::run()` can do and no test binary can. So even a caller that
//!    reached for the real implementation by mistake cannot modify the machine
//!    from a test (same guard the power controller and the recycle bin use).
//!
//! Reads through [`SystemRegistry`] are *not* guarded: they change nothing, and
//! the app has to be able to enumerate autostart entries.
//!
//! # Why values are carried as raw bytes
//!
//! A `Run` value can be `REG_SZ` or `REG_EXPAND_SZ`, and the difference matters
//! — `%ProgramFiles%\x\y.exe` only works as the latter. The startup manager
//! never rewrites a `Run` value (module spec 05: "Do NOT delete Run values"),
//! but the round-trip test proves that by comparing the stored type *and* the
//! stored bytes before and after a disable/re-enable cycle. Keeping the raw
//! form is what makes "byte-identical" a thing that can be asserted.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::core::error::{CoreError, Result};

/// The root keys this app touches.
///
/// `HKEY_CLASSES_ROOT` and friends are not here because nothing needs them; add
/// one when something does rather than exposing the whole registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Hive {
    /// `HKEY_CURRENT_USER` — this user's autostart entries.
    CurrentUser,
    /// `HKEY_LOCAL_MACHINE` — machine-wide entries. Writing here needs admin.
    LocalMachine,
}

impl Hive {
    /// Short name for logs and for the UI's "where this came from" line.
    pub fn short_name(self) -> &'static str {
        match self {
            Self::CurrentUser => "HKCU",
            Self::LocalMachine => "HKLM",
        }
    }
}

/// `REG_SZ`: a plain string.
pub const REG_TYPE_SZ: u32 = 1;
/// `REG_EXPAND_SZ`: a string with `%VARIABLES%` the shell expands.
pub const REG_TYPE_EXPAND_SZ: u32 = 2;
/// `REG_BINARY`: raw bytes, which is what `StartupApproved` uses.
pub const REG_TYPE_BINARY: u32 = 3;

/// One registry value, kept in the exact form the registry holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegValue {
    /// Value name. The default (unnamed) value is the empty string.
    pub name: String,
    /// The `REG_*` type code, preserved so a write puts back what was read.
    pub kind: u32,
    /// The raw bytes exactly as stored, NUL terminator included.
    pub bytes: Vec<u8>,
}

impl RegValue {
    /// A string value of the given type.
    ///
    /// Used by the fake registry and by tests; the real one never synthesises a
    /// string value, it only reads them.
    pub fn string(name: impl Into<String>, kind: u32, text: &str) -> Self {
        let mut bytes = Vec::with_capacity((text.len() + 1) * 2);
        for unit in text.encode_utf16().chain(std::iter::once(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        Self {
            name: name.into(),
            kind,
            bytes,
        }
    }

    /// A binary value.
    pub fn binary(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            kind: REG_TYPE_BINARY,
            bytes: bytes.into(),
        }
    }

    /// The value as text, when it is one of the string types.
    ///
    /// Returns `None` for anything else rather than guessing — a `Run` value
    /// that is not a string is not a command line, and pretending otherwise is
    /// how a garbage entry ends up in a list of "programs".
    pub fn as_string(&self) -> Option<String> {
        if self.kind != REG_TYPE_SZ && self.kind != REG_TYPE_EXPAND_SZ {
            return None;
        }
        Some(decode_utf16_le(&self.bytes))
    }

    /// Whether the value's text needs `%VARIABLE%` expansion before use.
    pub fn needs_expansion(&self) -> bool {
        self.kind == REG_TYPE_EXPAND_SZ
    }
}

/// Decode a little-endian UTF-16 buffer, stopping at the first NUL.
fn decode_utf16_le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Read and write the handful of registry locations this app cares about.
///
/// A key that does not exist is **not** an error: `values` returns an empty
/// vector and `value` returns `None`. A machine without `RunOnce` is normal, and
/// making the caller distinguish "absent" from "broken" at every call site is
/// how absent quietly becomes broken.
pub trait Registry: Send + Sync {
    /// Every value under `subkey`, in enumeration order.
    ///
    /// # Errors
    /// Returns an error when the key exists but cannot be read.
    fn values(&self, hive: Hive, subkey: &str) -> Result<Vec<RegValue>>;

    /// One value, or `None` when either the key or the value is absent.
    ///
    /// # Errors
    /// Returns an error when the key exists but cannot be read.
    fn value(&self, hive: Hive, subkey: &str, name: &str) -> Result<Option<RegValue>>;

    /// Write `value` under `subkey`, creating the key when it does not exist.
    ///
    /// # Errors
    /// Returns an error when the key cannot be created or the write is refused
    /// (writing under `HKLM` needs elevation).
    fn set_value(&self, hive: Hive, subkey: &str, value: &RegValue) -> Result<()>;
}

/* -------------------------------------------------------------------------
 * The fake
 * ---------------------------------------------------------------------- */

/// An in-memory registry for tests.
///
/// Keys are compared case-insensitively, as the real registry does. It records
/// every write so a test can assert on exactly what was stored — which is how
/// the `StartupApproved` byte layout is pinned without touching a real machine.
#[derive(Debug, Default)]
pub struct MapRegistry {
    keys: Mutex<BTreeMap<(Hive, String), Vec<RegValue>>>,
    writes: Mutex<Vec<(Hive, String, RegValue)>>,
    /// When set, every write fails with this message — the shape of an
    /// unelevated `HKLM` write.
    refuse_writes: Mutex<Option<String>>,
}

impl MapRegistry {
    /// An empty fake registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a key with values, replacing anything already there.
    #[must_use]
    pub fn with_key(self, hive: Hive, subkey: &str, values: Vec<RegValue>) -> Self {
        self.lock_keys().insert((hive, normalise(subkey)), values);
        self
    }

    /// Make every write fail, as an unelevated `HKLM` write does.
    #[must_use]
    pub fn refusing_writes(self, message: impl Into<String>) -> Self {
        *self
            .refuse_writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.into());
        self
    }

    /// Every write that has happened, in order.
    pub fn writes(&self) -> Vec<(Hive, String, RegValue)> {
        self.writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The values currently stored under `subkey`.
    pub fn snapshot(&self, hive: Hive, subkey: &str) -> Vec<RegValue> {
        self.lock_keys()
            .get(&(hive, normalise(subkey)))
            .cloned()
            .unwrap_or_default()
    }

    fn lock_keys(&self) -> std::sync::MutexGuard<'_, BTreeMap<(Hive, String), Vec<RegValue>>> {
        self.keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn normalise(subkey: &str) -> String {
    subkey.trim_matches('\\').to_lowercase()
}

impl Registry for MapRegistry {
    fn values(&self, hive: Hive, subkey: &str) -> Result<Vec<RegValue>> {
        Ok(self.snapshot(hive, subkey))
    }

    fn value(&self, hive: Hive, subkey: &str, name: &str) -> Result<Option<RegValue>> {
        Ok(self
            .snapshot(hive, subkey)
            .into_iter()
            .find(|value| value.name.eq_ignore_ascii_case(name)))
    }

    fn set_value(&self, hive: Hive, subkey: &str, value: &RegValue) -> Result<()> {
        if let Some(message) = self
            .refuse_writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Err(CoreError::Registry(message));
        }

        let mut keys = self.lock_keys();
        let entry = keys.entry((hive, normalise(subkey))).or_default();
        match entry
            .iter_mut()
            .find(|existing| existing.name.eq_ignore_ascii_case(&value.name))
        {
            Some(existing) => *existing = value.clone(),
            None => entry.push(value.clone()),
        }
        drop(keys);

        self.writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((hive, normalise(subkey), value.clone()));
        Ok(())
    }
}

/* -------------------------------------------------------------------------
 * The real thing
 * ---------------------------------------------------------------------- */

/// The registry of the machine this is running on.
///
/// Reads freely; **refuses to write** unless the process armed real OS effects,
/// which only the composition root can do. A test that somehow got hold of one
/// of these can enumerate but cannot change anything.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRegistry;

#[cfg(windows)]
mod sys {
    use super::{Hive, RegValue};
    use crate::core::error::{CoreError, Result};
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_VALUE_TYPE,
    };

    /// Longest value name Windows allows, plus the terminator.
    const MAX_VALUE_NAME: usize = 16_384;

    fn root(hive: Hive) -> HKEY {
        match hive {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// An open key handle that closes itself.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful open/create and is only
            // closed once, here.
            let _ = unsafe { RegCloseKey(self.0) };
        }
    }

    /// Open `subkey` for reading. `Ok(None)` when it does not exist.
    fn open_read(hive: Hive, subkey: &str) -> Result<Option<Key>> {
        let path = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: `path` is NUL-terminated and outlives the call; `handle` is a
        // valid out-pointer.
        let status = unsafe {
            RegOpenKeyExW(
                root(hive),
                PCWSTR(path.as_ptr()),
                None,
                KEY_READ,
                &mut handle,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(CoreError::Registry(format!(
                "cannot open `{}\\{subkey}` for reading: {}",
                hive.short_name(),
                status.0
            )));
        }
        Ok(Some(Key(handle)))
    }

    /// Open `subkey` for writing, creating it when absent.
    fn open_write(hive: Hive, subkey: &str) -> Result<Key> {
        let path = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: `path` is NUL-terminated and outlives the call; `handle` is a
        // valid out-pointer; the disposition out-parameter is not wanted.
        let status = unsafe {
            RegCreateKeyExW(
                root(hive),
                PCWSTR(path.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut handle,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(CoreError::Registry(format!(
                "cannot open `{}\\{subkey}` for writing: {} \
                 (writing under HKLM needs an elevated process)",
                hive.short_name(),
                status.0
            )));
        }
        Ok(Key(handle))
    }

    /// Enumerate every value under an open key.
    pub(super) fn values(hive: Hive, subkey: &str) -> Result<Vec<RegValue>> {
        let Some(key) = open_read(hive, subkey)? else {
            return Ok(Vec::new());
        };

        let mut out = Vec::new();
        let mut name = vec![0u16; MAX_VALUE_NAME];
        let mut index = 0u32;
        loop {
            let mut name_len = name.len() as u32;
            let mut data_len = 0u32;

            // First call sizes the data; `lpdata: None` asks for the length only,
            // and the type is re-read by the query below rather than kept here.
            // SAFETY: every pointer is to a live local, and `name_len` is the
            // real length of `name` in `u16`s.
            let status = unsafe {
                RegEnumValueW(
                    key.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut name_len,
                    None,
                    None,
                    None,
                    Some(&mut data_len),
                )
            };
            if status != ERROR_SUCCESS {
                // ERROR_NO_MORE_ITEMS ends the loop; anything else is a real
                // failure, but a half-read key is still better than nothing, so
                // it is logged and the enumeration stops.
                if status.0 != 259 {
                    tracing::warn!(
                        hive = hive.short_name(),
                        subkey,
                        code = status.0,
                        "registry enumeration stopped early"
                    );
                }
                break;
            }

            let value_name = String::from_utf16_lossy(&name[..name_len as usize]);
            let mut bytes = vec![0u8; data_len as usize];
            let mut read_len = data_len;
            let mut read_kind = REG_VALUE_TYPE::default();
            let value_name_wide = wide(&value_name);
            // SAFETY: the buffer is `read_len` bytes long, which is what the API
            // is told, and the name is NUL-terminated.
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    PCWSTR(value_name_wide.as_ptr()),
                    None,
                    Some(&mut read_kind),
                    Some(bytes.as_mut_ptr()),
                    Some(&mut read_len),
                )
            };
            if status == ERROR_SUCCESS {
                bytes.truncate(read_len as usize);
                out.push(RegValue {
                    name: value_name,
                    kind: read_kind.0,
                    bytes,
                });
            } else if status != ERROR_MORE_DATA {
                tracing::warn!(
                    hive = hive.short_name(),
                    subkey,
                    value = value_name.as_str(),
                    code = status.0,
                    "a registry value could not be read and was left out"
                );
            }

            index += 1;
        }
        Ok(out)
    }

    /// Read one value.
    pub(super) fn value(hive: Hive, subkey: &str, name: &str) -> Result<Option<RegValue>> {
        let Some(key) = open_read(hive, subkey)? else {
            return Ok(None);
        };
        let name_wide = wide(name);
        let mut kind = REG_VALUE_TYPE::default();
        let mut len = 0u32;
        // SAFETY: `name_wide` is NUL-terminated; asking for the size only.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(name_wide.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut len),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(CoreError::Registry(format!(
                "cannot read `{}\\{subkey}\\{name}`: {}",
                hive.short_name(),
                status.0
            )));
        }

        let mut bytes = vec![0u8; len as usize];
        let mut read_len = len;
        // SAFETY: the buffer really is `read_len` bytes long.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(name_wide.as_ptr()),
                None,
                Some(&mut kind),
                Some(bytes.as_mut_ptr()),
                Some(&mut read_len),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(CoreError::Registry(format!(
                "cannot read `{}\\{subkey}\\{name}`: {}",
                hive.short_name(),
                status.0
            )));
        }
        bytes.truncate(read_len as usize);
        Ok(Some(RegValue {
            name: name.to_owned(),
            kind: kind.0,
            bytes,
        }))
    }

    /// Write one value.
    pub(super) fn set_value(hive: Hive, subkey: &str, value: &RegValue) -> Result<()> {
        let key = open_write(hive, subkey)?;
        let name_wide = wide(&value.name);
        // SAFETY: `name_wide` is NUL-terminated and the slice is the data.
        let status = unsafe {
            RegSetValueExW(
                key.0,
                PCWSTR(name_wide.as_ptr()),
                None,
                REG_VALUE_TYPE(value.kind),
                Some(&value.bytes),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(CoreError::Registry(format!(
                "cannot write `{}\\{subkey}\\{}`: {}",
                hive.short_name(),
                value.name,
                status.0
            )));
        }
        Ok(())
    }
}

impl Registry for SystemRegistry {
    fn values(&self, hive: Hive, subkey: &str) -> Result<Vec<RegValue>> {
        #[cfg(windows)]
        {
            sys::values(hive, subkey)
        }
        #[cfg(not(windows))]
        {
            let _ = (hive, subkey);
            Ok(Vec::new())
        }
    }

    fn value(&self, hive: Hive, subkey: &str, name: &str) -> Result<Option<RegValue>> {
        #[cfg(windows)]
        {
            sys::value(hive, subkey, name)
        }
        #[cfg(not(windows))]
        {
            let _ = (hive, subkey, name);
            Ok(None)
        }
    }

    fn set_value(&self, hive: Hive, subkey: &str, value: &RegValue) -> Result<()> {
        // The same guard the power controller and the recycle bin use. A test
        // binary can never arm it, so this is the structural half of "tests must
        // never write to the real registry" (module spec 05).
        if !crate::core::platform::real_os_effects_armed() {
            return Err(CoreError::RegistryBlocked);
        }
        #[cfg(windows)]
        {
            sys::set_value(hive, subkey, value)
        }
        #[cfg(not(windows))]
        {
            let _ = (hive, subkey, value);
            Err(CoreError::Registry(
                "there is no registry on this platform".to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    #[test]
    fn a_string_value_round_trips_through_utf16() {
        let value = RegValue::string("OneDrive", REG_TYPE_SZ, "C:\\Program Files\\OneDrive.exe");
        assert_eq!(
            value.as_string().as_deref(),
            Some("C:\\Program Files\\OneDrive.exe")
        );
        assert!(!value.needs_expansion());
    }

    #[test]
    fn an_expand_string_says_it_needs_expanding() {
        let value = RegValue::string("App", REG_TYPE_EXPAND_SZ, "%ProgramFiles%\\App\\app.exe");
        assert!(value.needs_expansion());
        assert_eq!(
            value.as_string().as_deref(),
            Some("%ProgramFiles%\\App\\app.exe")
        );
    }

    #[test]
    fn a_binary_value_is_not_read_as_text() {
        let value = RegValue::binary("Flag", vec![2, 0, 0, 0]);
        assert_eq!(
            value.as_string(),
            None,
            "a binary blob is not a command line, and guessing that it is puts \
             garbage in front of the user"
        );
    }

    #[test]
    fn a_missing_key_reads_as_empty_rather_than_failing() {
        let registry = MapRegistry::new();
        assert!(registry
            .values(Hive::CurrentUser, RUN)
            .expect("read")
            .is_empty());
        assert_eq!(
            registry
                .value(Hive::CurrentUser, RUN, "Nothing")
                .expect("read"),
            None
        );
    }

    #[test]
    fn keys_and_value_names_are_matched_case_insensitively() {
        let registry = MapRegistry::new().with_key(
            Hive::CurrentUser,
            RUN,
            vec![RegValue::string("OneDrive", REG_TYPE_SZ, "x.exe")],
        );
        assert_eq!(
            registry
                .values(Hive::CurrentUser, &RUN.to_uppercase())
                .expect("read")
                .len(),
            1
        );
        assert!(registry
            .value(Hive::CurrentUser, RUN, "onedrive")
            .expect("read")
            .is_some());
    }

    #[test]
    fn the_two_hives_do_not_answer_for_each_other() {
        let registry = MapRegistry::new().with_key(
            Hive::LocalMachine,
            RUN,
            vec![RegValue::string("Machine", REG_TYPE_SZ, "x.exe")],
        );
        assert_eq!(
            registry
                .values(Hive::LocalMachine, RUN)
                .expect("read")
                .len(),
            1
        );
        assert!(registry
            .values(Hive::CurrentUser, RUN)
            .expect("read")
            .is_empty());
    }

    #[test]
    fn a_write_replaces_an_existing_value_and_is_recorded() {
        let registry = MapRegistry::new().with_key(
            Hive::CurrentUser,
            RUN,
            vec![RegValue::string("App", REG_TYPE_SZ, "old.exe")],
        );
        registry
            .set_value(
                Hive::CurrentUser,
                RUN,
                &RegValue::string("App", REG_TYPE_SZ, "new.exe"),
            )
            .expect("write");

        let stored = registry.snapshot(Hive::CurrentUser, RUN);
        assert_eq!(stored.len(), 1, "a write replaces, it does not duplicate");
        assert_eq!(stored[0].as_string().as_deref(), Some("new.exe"));
        assert_eq!(registry.writes().len(), 1);
    }

    #[test]
    fn a_refusing_registry_reports_the_reason() {
        let registry = MapRegistry::new().refusing_writes("access is denied");
        let err = registry
            .set_value(Hive::LocalMachine, RUN, &RegValue::binary("x", vec![2]))
            .expect_err("a refused write must surface");
        assert!(err.to_string().contains("access is denied"), "got {err}");
        assert!(registry.writes().is_empty(), "nothing must be recorded");
    }

    /// The structural half of "tests never write to the real registry": the real
    /// implementation refuses, because a test process cannot arm OS effects.
    #[test]
    fn the_real_registry_refuses_to_write_from_a_test_process() {
        assert!(!crate::core::platform::real_os_effects_armed());
        let err = SystemRegistry
            .set_value(
                Hive::CurrentUser,
                RUN,
                &RegValue::string("WinKnifeTest", REG_TYPE_SZ, "never.exe"),
            )
            .expect_err("a test build must not be able to write to the registry");
        assert!(matches!(err, CoreError::RegistryBlocked));
    }
}
