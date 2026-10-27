//! Paths that can never be rated Safe and never receive a delete action.
//!
//! Why this is code and not rule data: a rule pack is community-editable JSON.
//! If the protected list lived there, a well-meaning pull request could hand
//! someone a green "clean" button pointed at `C:\Windows`. The list is compiled
//! in, checked before any rule is consulted, and re-checked by the executor
//! (CONTRIBUTING.md hard rules, ARCHITECTURE section 4).
//!
//! Two scopes, and the difference matters:
//!
//! - [`Scope::Tree`] — the directory and everything under it. `C:\Windows`,
//!   `C:\Program Files`, our own quarantine.
//! - [`Scope::Exact`] — the directory itself but not its children. The user
//!   profile root (`C:\Users\dev` is protected, `C:\Users\dev\Downloads` is
//!   fair game) and drive roots, which would otherwise protect everything.
//!
//! # Spelling a protected path differently is not a way past it
//!
//! Win32 accepts many spellings of the same file, and a component comparison
//! against the text it was handed reads them as different places (security
//! review C1, C2):
//!
//! - `\\?\C:\Windows` and `\\.\C:\Windows` — the verbatim and device prefixes,
//!   which the object manager strips before it opens anything.
//! - `\??\C:\Windows` — the NT object namespace, same file again.
//! - `\\localhost\C$\Windows`, `\\127.0.0.1\C$\Windows` — the administrative
//!   share, which resolves from an ordinary non-elevated process and lands on
//!   the local disk. Canonicalising it produces `\\?\UNC\localhost\c$\...`,
//!   which is a *third* spelling.
//! - `\\?\GLOBALROOT\Device\HarddiskVolume3\Windows` — the volume by device
//!   name rather than by drive letter.
//! - `C:Windows\System32` — drive-relative, resolved against a per-drive
//!   current directory this process does not control.
//! - `C:\Windows.` and `C:\Windows ` — Win32 discards trailing dots and spaces
//!   in a path component, so both open `C:\Windows`.
//! - `%TEMP%\..\..\..\Windows` — `..` climbing back out of the folder a rule
//!   was pointed at.
//! - `C:\PROGRA~1` — the 8.3 alias of `C:\Program Files`.
//!
//! **Two rounds of rejecting bad spellings by name both shipped a bypass, so
//! that approach is closed.** The gate is now an allow-list: [`gate_path`] is
//! the single chokepoint every caller goes through, and it accepts exactly one
//! shape — a plain local drive-letter path, `X:\...`, canonicalised. Every
//! other shape is *refused as unjudgeable*, which is not the same as allowed:
//! the caller skips the item and reports why.
//!
//! This list stays, as defence in depth for the cases [`gate_path`] hands back
//! (`Local` and `Missing`) and for callers that only have text — but it is no
//! longer the only thing between a rule pack and `C:\Windows`.
//!
//! # The list's own spelling counts too
//!
//! An allow-list gate that canonicalises the path it is asked about only helps
//! if the entries it compares against are in the same spelling. `%TEMP%` on a
//! great many real accounts is `C:\Users\USERNA~1\AppData\Local\Temp`, and
//! `%WINDIR%`, `%USERPROFILE%` and `%ProgramFiles%` can arrive in 8.3 form too;
//! a list built from those variables stored the short name while the gate asked
//! about the long one, and the comparison missed. Every entry is therefore
//! registered in both spellings on the way in — see [`widen`].

use std::path::{Path, PathBuf};

use crate::core::path_compare::{self, CanonicalPath};
use crate::core::platform::EnvResolver;

/// Why a path that climbs with `..` is refused outright.
const TRAVERSAL_REASON: &str =
    "this path climbs out of its own folder with `..`, which can land anywhere, so it is never treated as cleanable";

/// Why a path that is not a plain local drive-letter path is refused outright.
///
/// One constant rather than a per-shape sentence because [`ProtectedPaths::reason_for`]
/// hands back a borrowed `&str`; [`gate_path`], which owns its message, says
/// exactly which shape was seen.
const UNJUDGEABLE_REASON: &str =
    "not a plain local drive-letter path — a UNC share, a device or NT-namespace path, or a relative one — so it cannot be compared with the protected list and is never treated as cleanable";

/// How far a protected entry reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// This path only; children are not protected.
    Exact,
    /// This path and everything beneath it.
    Tree,
}

/// One protected location and why it is protected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedEntry {
    /// The path, already normalised for comparison.
    pub path: PathBuf,
    /// How far the protection reaches.
    pub scope: Scope,
    /// Shown to the user verbatim, so it has to read like a sentence.
    pub reason: String,
}

/// The compiled-in protected-path list.
#[derive(Debug, Clone, Default)]
pub struct ProtectedPaths {
    entries: Vec<ProtectedEntry>,
    /// Each entry's path already split into comparable components, index for
    /// index with `entries`. [`ProtectedPaths::reason_for`] runs for every file
    /// a scan examines, and re-splitting every entry on every call was most of
    /// its cost on a 100 000-file `%TEMP%`. Same split, done once.
    entry_parts: Vec<Vec<String>>,
}

impl ProtectedPaths {
    /// An empty list. Only for tests that are about something else — the real
    /// app always builds one from [`ProtectedPaths::from_env`].
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from explicit entries. Every path is normalised *and* registered in
    /// its canonical spelling on the way in, so a caller cannot register an
    /// entry in a spelling the probe never matches (see [`widen`]).
    pub fn new(entries: Vec<ProtectedEntry>) -> Self {
        let mut widened = Vec::with_capacity(entries.len());
        for entry in entries {
            widen(entry, &mut widened);
        }
        let entry_parts = widened.iter().map(|entry| split(&entry.path)).collect();
        Self {
            entries: widened,
            entry_parts,
        }
    }

    /// Build a tree-scoped list from paths, with a generic reason. Used by
    /// fixture tests that plant a `C:\Windows` stand-in inside a tempdir.
    pub fn from_trees(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self::new(
            paths
                .into_iter()
                .map(|path| ProtectedEntry {
                    reason: format!("`{}` is a protected system location", path.display()),
                    path: normalise(&path),
                    scope: Scope::Tree,
                })
                .collect(),
        )
    }

    /// The real Windows list, resolved through `env` so tests can point it
    /// somewhere harmless.
    ///
    /// Environment variables only ever *widen* this list. The default install
    /// locations are compiled in unconditionally, because an environment where
    /// `%WINDIR%` is unset or has been pointed somewhere harmless is exactly
    /// the environment in which an unprotected `C:\Windows` does the most
    /// damage (security review C1e).
    ///
    /// `quarantine_root` and `app_data_dir` are ours: cleaning our own storage
    /// would let the app eat the very files it promised to keep restorable.
    pub fn from_env(env: &dyn EnvResolver, app_data_dir: &Path, quarantine_root: &Path) -> Self {
        let mut entries = Vec::new();

        let windows_reason =
            "the Windows directory: removing anything here can stop the system booting";
        let programs_reason =
            "an installed-programs directory: use the app's own uninstaller instead";
        let shared_reason = "shared application data used by installed programs";

        let mut tree = |value: Option<String>, reason: &str| {
            if let Some(value) = value {
                let path = PathBuf::from(value);
                if !path.as_os_str().is_empty() {
                    entries.push(ProtectedEntry {
                        path: normalise(&path),
                        scope: Scope::Tree,
                        reason: reason.to_owned(),
                    });
                }
            }
        };

        // The compiled-in floor. These are the stock locations on every
        // Windows install; a machine that has moved them still gets them from
        // the variables below.
        tree(Some("C:\\Windows".to_owned()), windows_reason);
        tree(Some("C:\\Program Files".to_owned()), programs_reason);
        tree(Some("C:\\Program Files (x86)".to_owned()), programs_reason);
        tree(Some("C:\\ProgramData".to_owned()), shared_reason);

        tree(
            env.var("WINDIR").or_else(|| env.var("SystemRoot")),
            windows_reason,
        );
        tree(env.var("ProgramFiles"), programs_reason);
        tree(env.var("ProgramFiles(x86)"), programs_reason);
        tree(env.var("ProgramW6432"), programs_reason);
        tree(env.var("ProgramData"), shared_reason);

        entries.push(ProtectedEntry {
            path: normalise(app_data_dir),
            scope: Scope::Tree,
            reason: "this app's own data directory".to_owned(),
        });
        entries.push(ProtectedEntry {
            path: normalise(quarantine_root),
            scope: Scope::Tree,
            reason: "the quarantine: this is what makes cleaning reversible".to_owned(),
        });

        // The profile root itself, but not what is inside it — Downloads and
        // AppData have to stay reachable for later phases.
        if let Some(profile) = env.var("USERPROFILE") {
            entries.push(ProtectedEntry {
                path: normalise(Path::new(&profile)),
                scope: Scope::Exact,
                reason: "your user profile folder itself".to_owned(),
            });
        }

        // Drive roots, exact-scope: `C:\` protected, `C:\Temp` not.
        for letter in 'a'..='z' {
            entries.push(ProtectedEntry {
                path: PathBuf::from(format!("{letter}:")),
                scope: Scope::Exact,
                reason: "a drive root".to_owned(),
            });
        }

        // Volume metadata Windows manages itself; matched by name at any depth
        // because it exists once per volume.
        entries.push(ProtectedEntry {
            path: PathBuf::from("system volume information"),
            scope: Scope::Tree,
            reason: "volume metadata managed by Windows".to_owned(),
        });

        Self::new(entries)
    }

    /// Add one more entry (used to register the quarantine root once it is
    /// known). Widened exactly as [`ProtectedPaths::new`] widens.
    pub fn push(&mut self, entry: ProtectedEntry) {
        widen(entry, &mut self.entries);
        self.entry_parts = self
            .entries
            .iter()
            .map(|entry| split(&entry.path))
            .collect();
    }

    /// Every entry, for tests and diagnostics.
    pub fn entries(&self) -> &[ProtectedEntry] {
        &self.entries
    }

    /// Why `path` is protected, or `None` when it is not.
    ///
    /// Comparison is case-insensitive, separator-agnostic and blind to the
    /// verbatim prefix and the trailing dots and spaces Win32 discards, and it
    /// works on paths that do not exist so the rules can be tested without a
    /// filesystem.
    ///
    /// A path that still climbs with `..` is reported as protected: this list
    /// resolves `..` lexically, and a lexical answer is the wrong one as soon
    /// as a junction is involved, so the honest verdict is "not judgeable".
    /// Nothing legitimate reaches here with a `..` in it — the walker produces
    /// absolute paths and the rule-pack parser rejects the pattern outright.
    ///
    /// # Why this one comparison stays lexical
    ///
    /// Both sides are compared as normalised text, not resolved against the
    /// disk, and that is deliberate. This runs one probe against every entry in
    /// the list, for every file examined — resolving per entry would be a
    /// syscall per entry per file. The disk lookup happens once instead, in the
    /// two places that can afford it: [`widen`] resolves each entry when the
    /// list is built, and [`Brain::path_refusal`] resolves the probe before it
    /// asks. So by the time a comparison reaches here, both sides have already
    /// been through [`crate::core::path_compare`].
    ///
    /// [`Brain::path_refusal`]: crate::core::brain::Brain::path_refusal
    pub fn reason_for(&self, path: &Path) -> Option<&str> {
        if has_traversal(path) {
            return Some(TRAVERSAL_REASON);
        }
        // Anything that is not anchored on a drive letter cannot be compared
        // with entries that all are, and "no entry matched" used to be reported
        // as "not protected" (security review C2e). Unjudgeable is refused.
        if plain_local_shape(path).is_err() {
            return Some(UNJUDGEABLE_REASON);
        }

        let candidate_components: Vec<_> = split(path);

        for (entry, entry_components) in self.entries.iter().zip(&self.entry_parts) {
            if entry_components.is_empty() {
                continue;
            }

            let matched = match entry.scope {
                Scope::Exact => candidate_components == *entry_components,
                Scope::Tree => {
                    // A single-component tree entry with no drive letter (the
                    // volume metadata folder) matches wherever it appears.
                    if entry_components.len() == 1 && !entry_components[0].ends_with(':') {
                        candidate_components
                            .iter()
                            .any(|part| part == &entry_components[0])
                    } else {
                        candidate_components.len() >= entry_components.len()
                            && candidate_components[..entry_components.len()]
                                == entry_components[..]
                    }
                }
            };

            if matched {
                return Some(&entry.reason);
            }
        }
        None
    }

    /// Whether `path` is protected.
    pub fn is_protected(&self, path: &Path) -> bool {
        self.reason_for(path).is_some()
    }
}

/// What [`gate_path`] concluded about a path.
///
/// There is no "allowed" variant that skips the protected-path list: `Local`
/// hands back the canonical form precisely so the caller matches *that* against
/// the list rather than the text it was given.
#[derive(Debug)]
pub enum GateVerdict {
    /// A plain local drive-letter path that exists, canonicalised — 8.3
    /// aliases, junctions, symlinks and the verbatim prefix already resolved
    /// away. The caller must still check it against [`ProtectedPaths`].
    Local(PathBuf),
    /// The shape is judgeable, but nothing is there. Safe to carry on: a walk
    /// finds nothing and a move reports the miss.
    Missing,
    /// Not judgeable, and therefore not usable. The caller skips the item and
    /// reports this clause, which is phrased to follow the word "because".
    Refused(String),
}

/// The one gate every path crosses before anything is walked or moved.
///
/// It answers a single question — "is this a plain local drive-letter path, and
/// where does it really point?" — and refuses everything else. Both the walk
/// root check (`modules::cleaner::resolve_root`) and the execute-time check
/// (`core::executor`) go through here; neither may fall back to the raw string.
///
/// Why an allow-list: `\\localhost\C$\Windows\System32\ntoskrnl.exe` is
/// `C:\Windows\System32\ntoskrnl.exe`, resolves from a non-elevated process,
/// and canonicalises to `\\?\UNC\localhost\c$\...` — a spelling neither the
/// lexical list nor its canonicalising callers recognised. Rejecting that
/// spelling by name would only invite the next one (security review C2).
pub fn gate_path(path: &Path) -> GateVerdict {
    if let Err(why) = plain_local_shape(path) {
        return GateVerdict::Refused(format!(
            "`{}` is not a plain local drive-letter path ({why}), so it cannot be checked \
             against the protected list and is treated as protected",
            path.display()
        ));
    }

    match dunce::canonicalize(path) {
        Ok(canonical) => {
            // Canonicalising is what turns a junction, a symlink or an 8.3
            // alias into the place it really opens — and a local path can
            // resolve *onto* a UNC share, so the shape has to be asked of the
            // answer as well as of the question.
            if let Err(why) = plain_local_shape(&canonical) {
                return GateVerdict::Refused(format!(
                    "`{}` resolves to `{}`, which is not a plain local drive-letter path \
                     ({why}), so it is treated as protected",
                    path.display(),
                    canonical.display()
                ));
            }
            GateVerdict::Local(canonical)
        }
        // Nothing is there to protect. The shape was already checked above, so
        // this is a real "not found", not an unjudgeable spelling.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => GateVerdict::Missing,
        Err(err) => GateVerdict::Refused(format!(
            "`{}` could not be resolved to a real location ({err}), so it is treated as protected",
            path.display()
        )),
    }
}

/// Whether `path` is a plain local drive-letter path, and which shape it is
/// when it is not.
///
/// The entire allow-list lives here. Nothing is matched by name: a path is
/// judgeable only if, after an optional `\\?\` verbatim prefix, its first
/// component is `X:` and a separator follows. UNC (`\\server\share`,
/// `\\?\UNC\...`), the device namespace (`\\.\`, `\\?\GLOBALROOT\...`), the NT
/// object namespace (`\??\`), drive-relative (`C:System32`), rooted without a
/// drive (`\Windows`), relative, empty and non-Unicode paths all fail, because
/// none of them can be compared with a list of `X:\...` entries.
///
/// The returned clause is phrased to follow "because".
///
/// Deliberately a *text* check, and not routed through
/// [`crate::core::path_compare`]: "how is this written?" is a question about the
/// text by definition, and this runs before anything is resolved — refusing an
/// unjudgeable spelling without touching it is the whole point of an allow-list.
fn plain_local_shape(path: &Path) -> std::result::Result<(), String> {
    let Some(text) = path.to_str() else {
        return Err("it is not valid Unicode, so it has no comparable spelling".to_owned());
    };
    if text.trim().is_empty() {
        return Err("it is empty".to_owned());
    }
    let text = text.replace('/', "\\");

    if text.split('\\').any(|part| part == "..") {
        return Err("it climbs with `..`, which can land anywhere".to_owned());
    }
    if text.starts_with("\\??\\") {
        return Err("it uses the `\\??\\` NT object namespace".to_owned());
    }
    if text.starts_with("\\\\.\\") {
        return Err("it uses the `\\\\.\\` device namespace".to_owned());
    }

    let rest = match text.strip_prefix("\\\\?\\") {
        // `\\?\UNC\...` and `\\?\GLOBALROOT\...` come back with a first
        // component that is not a drive, and fall out of the check below.
        Some(rest) => rest,
        None if text.starts_with("\\\\") => {
            return Err(
                "it is a UNC network path, and `\\\\host\\C$` reaches the local disk under a \
                 name the protected list cannot see"
                    .to_owned(),
            )
        }
        None => text.as_str(),
    };

    if rest.starts_with('\\') {
        return Err(
            "it is rooted but names no drive, so which volume it means is ambiguous".to_owned(),
        );
    }

    let bytes = rest.as_bytes();
    let first = rest.split('\\').next().unwrap_or_default();
    if bytes.len() < 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return Err(format!(
            "its first component `{first}` is not a drive letter such as `C:`"
        ));
    }
    if bytes.len() == 2 || bytes[2] != b'\\' {
        return Err(
            "it is drive-relative (`C:name`), which resolves against a per-drive current \
             directory this process does not control"
                .to_owned(),
        );
    }
    Ok(())
}

/// Register one entry in every spelling the comparison has to recognise: the
/// text it was written in, normalised, and — when the filesystem spells that
/// same directory differently — its canonical form as well.
///
/// # Why an entry needs both spellings
///
/// `%TEMP%` on a great many real accounts is `C:\Users\USERNA~1\AppData\Local\
/// Temp`, and `%WINDIR%`, `%USERPROFILE%` and `%ProgramFiles%` can reach us in
/// 8.3 form too. A list built from those variables stored the short spelling,
/// while [`gate_path`] canonicalised the path it was asked about and produced
/// the long one. Two spellings of one directory, compared component by
/// component, do not match — so a protected location quietly stopped being
/// protected on exactly the machines whose environment is spelled that way.
/// (CI found this: the GitHub runner's temp directory is `C:\Users\RUNNER~1\…`.)
///
/// Both spellings are kept rather than only the canonical one, because the two
/// sides of the comparison do not always agree on which is which:
///
/// - The gate canonicalises, so it asks in the long form. The canonical entry
///   is what answers it.
/// - [`ProtectedPaths::reason_for`] is also the defence-in-depth layer for
///   callers that only ever have text — a rule pack's own spelling, a path from
///   the database — and that text may well be the short form the variable was
///   in. The normalised raw entry is what answers *that*.
///
/// Adding a spelling only ever protects more, which is the direction to be
/// wrong in. Nothing is ever dropped: a directory this machine does not have
/// cannot be canonicalised, and losing the entry would turn a missing folder
/// into an unprotected one.
///
/// Only a plain local drive-letter path is canonicalised. A drive root entry is
/// `C:`, whose canonical form is that drive's *current directory* — a real path
/// and the wrong one to protect exactly — and the volume-metadata entry is a
/// bare name that belongs to no drive at all.
fn widen(entry: ProtectedEntry, into: &mut Vec<ProtectedEntry>) {
    let normalised = normalise(&entry.path);

    // Only a plain local drive-letter path is resolved. A drive root entry is
    // `C:`, and a bare name such as the volume-metadata folder belongs to no
    // drive at all — resolving either would ask the disk about somewhere else.
    let canonical = plain_local_shape(&normalised)
        .ok()
        .map(|()| CanonicalPath::resolved(&normalised).to_path_buf())
        .filter(|canonical| canonical != &normalised);

    if let Some(canonical) = canonical {
        into.push(ProtectedEntry {
            path: canonical,
            scope: entry.scope,
            reason: entry.reason.clone(),
        });
    }
    into.push(ProtectedEntry {
        path: normalised,
        ..entry
    });
}

/// The canonical text of a path for comparison purposes: lower-cased,
/// backslash-separated, no verbatim prefix, no trailing separator.
///
/// Not `canonicalize`: that needs the path to exist, and the brain has to be
/// able to judge a path it has only been told about. [`widen`] resolves each
/// entry against the disk on the way in, and the gate resolves the probe, so
/// both sides of a `reason_for` comparison have already been through
/// [`crate::core::path_compare`] by the time they meet.
fn normalise(path: &Path) -> PathBuf {
    CanonicalPath::lexical(path).to_path_buf()
}

/// Split a path into comparable components. `C:\Windows\System32` becomes
/// `["c:", "windows", "system32"]`.
///
/// The Windows path text semantics — verbatim prefix, trailing dot and space,
/// lexical `..`, the UNC share root that is one component and not two — live in
/// [`crate::core::path_compare`], because three separate copies of them is how
/// two spellings of one path came to be compared as raw text three reviews
/// running.
fn split(path: &Path) -> Vec<String> {
    CanonicalPath::lexical(path).parts().to_vec()
}

/// Whether a path contains a `..` component in any spelling.
fn has_traversal(path: &Path) -> bool {
    path_compare::has_traversal(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::MapEnv;

    fn windows_like() -> ProtectedPaths {
        let env = MapEnv::new()
            .with("WINDIR", "C:\\Windows")
            .with("ProgramFiles", "C:\\Program Files")
            .with("ProgramFiles(x86)", "C:\\Program Files (x86)")
            .with("ProgramData", "C:\\ProgramData")
            .with("USERPROFILE", "C:\\Users\\dev");
        ProtectedPaths::from_env(
            &env,
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife"),
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife\\quarantine"),
        )
    }

    /// Table-driven, as CONVENTIONS.md requires for path classification.
    #[test]
    fn protected_path_table() {
        let protected = windows_like();
        let cases: &[(&str, bool, &str)] = &[
            ("C:\\Windows", true, "the Windows directory itself"),
            (
                "C:\\Windows\\System32\\ntoskrnl.exe",
                true,
                "deep inside Windows",
            ),
            ("c:\\windows\\temp\\a.tmp", true, "case-insensitive match"),
            ("C:/Windows/System32", true, "forward slashes normalise"),
            ("C:\\Windows\\", true, "a trailing separator is ignored"),
            (
                "C:\\Program Files\\App\\app.exe",
                true,
                "installed programs",
            ),
            (
                "C:\\Program Files (x86)\\Old\\x.dll",
                true,
                "32-bit programs",
            ),
            ("C:\\ProgramData\\Vendor\\cache.db", true, "shared app data"),
            ("C:\\Users\\dev", true, "the profile root itself"),
            ("C:\\USERS\\DEV\\", true, "profile root, different case"),
            (
                "C:\\Users\\dev\\Downloads",
                false,
                "a profile subdirectory is not",
            ),
            (
                "C:\\Users\\dev\\AppData\\Local\\Temp\\a.tmp",
                false,
                "the temp folder must stay cleanable",
            ),
            (
                "C:\\Users\\dev\\AppData\\Roaming\\WinKnife\\winknife.sqlite3",
                true,
                "our own data",
            ),
            (
                "C:\\Users\\dev\\AppData\\Roaming\\WinKnife\\quarantine\\x\\y",
                true,
                "the quarantine itself",
            ),
            ("C:\\", true, "a drive root"),
            ("D:\\", true, "any drive root"),
            ("C:\\Temp\\a.log", false, "a top-level folder is not a root"),
            (
                "D:\\System Volume Information\\x",
                true,
                "volume metadata at any depth",
            ),
            (
                "C:\\Windows.old\\x",
                false,
                "a similar prefix is not a match",
            ),
            (
                "C:\\WindowsApps-backup\\x",
                false,
                "component matching, not string prefix matching",
            ),
            (
                "C:\\Users\\dev2",
                false,
                "a sibling profile is not the profile",
            ),
        ];

        for (path, expected, why) in cases {
            assert_eq!(
                protected.is_protected(Path::new(path)),
                *expected,
                "{path} — {why}"
            );
        }
    }

    #[test]
    fn every_protected_path_explains_itself() {
        let protected = windows_like();
        let reason = protected
            .reason_for(Path::new("C:\\Windows\\System32"))
            .expect("a protected path must carry a reason");
        assert!(!reason.trim().is_empty());
        assert!(reason.len() > 10, "the reason must read like a sentence");
    }

    #[test]
    fn an_empty_list_protects_nothing() {
        assert!(!ProtectedPaths::empty().is_protected(Path::new("C:\\Windows")));
    }

    #[test]
    fn from_trees_protects_the_tree_and_nothing_beside_it() {
        let protected = ProtectedPaths::from_trees([PathBuf::from("C:\\fixture\\Windows")]);
        assert!(protected.is_protected(Path::new("C:\\fixture\\Windows")));
        assert!(protected.is_protected(Path::new("C:\\fixture\\Windows\\System32\\a.dll")));
        assert!(!protected.is_protected(Path::new("C:\\fixture\\Temp\\a.tmp")));
        assert!(!protected.is_protected(Path::new("C:\\fixture\\WindowsApps")));
    }

    /// Environment variables widen the list; they never replace it. An
    /// environment that has lost `%WINDIR%` must still protect `C:\Windows`
    /// (security review C1e).
    #[test]
    fn missing_environment_variables_fall_back_to_the_compiled_in_list() {
        let protected = ProtectedPaths::from_env(
            &MapEnv::new(),
            Path::new("C:\\app"),
            Path::new("C:\\app\\q"),
        );

        for path in [
            "C:\\Windows\\System32",
            "C:\\Program Files\\App\\a.exe",
            "C:\\Program Files (x86)\\Old\\x.dll",
            "C:\\ProgramData\\Vendor\\cache.db",
            "C:\\",
            "C:\\app\\q\\batch\\item",
        ] {
            assert!(
                protected.is_protected(Path::new(path)),
                "`{path}` must be protected even with an empty environment"
            );
        }
        // Widening still works: a machine with Windows somewhere else says so.
        let moved = ProtectedPaths::from_env(
            &MapEnv::new().with("WINDIR", "D:\\Windows"),
            Path::new("C:\\app"),
            Path::new("C:\\app\\q"),
        );
        assert!(moved.is_protected(Path::new("D:\\Windows\\System32")));
        assert!(moved.is_protected(Path::new("C:\\Windows\\System32")));
    }

    /// Security review C1: every spelling of a protected path that Win32
    /// accepts has to be refused, not just the one the list was written in.
    #[test]
    fn alternate_spellings_of_a_protected_path_are_still_protected() {
        let protected = windows_like();
        let cases: &[(&str, &str)] = &[
            (
                "C:\\Users\\dev\\AppData\\Local\\Temp\\..\\..\\..\\..\\Windows\\System32\\ntoskrnl.exe",
                "`..` traversal out of the folder a rule was pointed at",
            ),
            (
                "\\\\?\\C:\\Windows\\System32\\ntoskrnl.exe",
                "the verbatim prefix the object manager strips",
            ),
            (
                "\\\\.\\C:\\Windows\\System32\\ntoskrnl.exe",
                "the device-namespace prefix",
            ),
            ("C:\\Windows.\\System32\\x", "a trailing dot Win32 discards"),
            (
                "C:\\Windows \\System32\\x",
                "a trailing space Win32 discards",
            ),
            (
                "C:\\Program Files.\\App\\a.exe",
                "trailing dot on an installed-programs directory",
            ),
            (
                "\\\\?\\C:\\Users\\dev",
                "the verbatim spelling of the profile root",
            ),
            (
                "C:/Users/dev/Downloads/../../../Windows",
                "forward slashes and `..` together",
            ),
        ];

        for (path, why) in cases {
            assert!(protected.is_protected(Path::new(path)), "{path} — {why}");
            assert!(
                protected
                    .reason_for(Path::new(path))
                    .is_some_and(|reason| reason.len() > 10),
                "{path} must still explain itself"
            );
        }
    }

    /// The 8.3 alias cannot be expanded without asking the filesystem, so this
    /// list deliberately does not catch it. The executor canonicalises before
    /// it consults the list, and `executor::tests` proves that path.
    #[test]
    fn an_8_3_alias_is_left_for_the_canonicalising_caller() {
        let protected = windows_like();
        assert!(!protected.is_protected(Path::new("C:\\PROGRA~1\\App\\a.exe")));
        assert!(
            protected.is_protected(Path::new("C:\\Program Files\\App\\a.exe")),
            "the long name it canonicalises to is what the gate sees"
        );
    }

    /// A directory with two real spellings: its 8.3 alias and its canonical
    /// long name. `None` when nothing on this volume mints an alias, so the
    /// caller can say so out loud rather than assert on a fixture that is not
    /// there.
    #[cfg(windows)]
    fn aliased_dir(parent: &Path, name: &str) -> Option<(PathBuf, PathBuf)> {
        let long = parent.join(name);
        std::fs::create_dir_all(&long).expect("fixture dir");
        let canonical = dunce::canonicalize(&long).expect("canonicalize");
        let short = crate::core::test_support::short_path_name(&long)?;
        (short != canonical).then_some((short, canonical))
    }

    /// The bug CI found, pinned: an entry registered in one spelling has to
    /// match a probe in the other, because they are one directory.
    ///
    /// `%TEMP%` really is `C:\Users\USERNA~1\AppData\Local\Temp` on a great many
    /// accounts — the GitHub runner's is — so a list built from such a variable
    /// stored the short spelling while [`gate_path`] canonicalised the probe and
    /// produced the long one. The comparison missed, and a protected location
    /// silently stopped being protected.
    #[cfg(windows)]
    #[test]
    fn an_entry_spelled_as_an_8_3_alias_still_matches_the_canonical_probe() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A long name with spaces is what makes Windows mint an alias.
        let Some((short, canonical)) = aliased_dir(dir.path(), "Protected Program Files") else {
            crate::core::test_support::privileged_operation_unavailable(
                "an_entry_spelled_as_an_8_3_alias_still_matches_the_canonical_probe",
                "no 8.3 alias is available here (fsutil 8dot3name), so the two spellings \
                 this test compares cannot be produced",
            );
            return;
        };

        let protected = ProtectedPaths::from_trees([short.clone()]);
        let victim = canonical.join("kernel.sys");
        assert!(
            protected.is_protected(&victim),
            "registered `{}`, probed `{}` — one directory, two spellings, and the gate \
             always hands the canonical one to the list. Entries: {:?}",
            short.display(),
            victim.display(),
            protected
                .entries()
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect::<Vec<_>>()
        );
        // And the short spelling keeps working, for the callers that only ever
        // have the text a rule pack was written in.
        assert!(protected.is_protected(&short.join("kernel.sys")));
    }

    /// The same thing where it actually bites in production: `%WINDIR%`,
    /// `%USERPROFILE%` and `%ProgramFiles%` can all be handed to us in 8.3 form,
    /// and the list the app runs on is built from exactly those.
    #[cfg(windows)]
    #[test]
    fn environment_variables_spelled_as_8_3_aliases_still_protect_the_real_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let Some((short_windows, canonical_windows)) = aliased_dir(dir.path(), "Windows Stand In")
        else {
            crate::core::test_support::privileged_operation_unavailable(
                "environment_variables_spelled_as_8_3_aliases_still_protect_the_real_directory",
                "no 8.3 alias is available here (fsutil 8dot3name), so the two spellings \
                 this test compares cannot be produced",
            );
            return;
        };
        let (short_profile, canonical_profile) =
            aliased_dir(dir.path(), "Profile Stand In").expect("the same volume, the same setting");

        let protected = ProtectedPaths::from_env(
            &MapEnv::new()
                .with("WINDIR", short_windows.to_string_lossy())
                .with("USERPROFILE", short_profile.to_string_lossy()),
            Path::new("C:\\app"),
            Path::new("C:\\app\\q"),
        );

        assert!(
            protected.is_protected(&canonical_windows.join("System32\\ntoskrnl.exe")),
            "a `%WINDIR%` handed to us in 8.3 form must still protect the tree it names"
        );
        assert!(
            protected.is_protected(&canonical_profile),
            "the same for the profile root, which is exact-scoped"
        );
        assert!(
            !protected.is_protected(&canonical_profile.join("Downloads")),
            "widening the profile entry must not turn its exact scope into a tree"
        );
    }

    /// Canonicalising an entry must not invent one. A drive root is `C:`, whose
    /// canonical form is that drive's *current directory* — a real path, and the
    /// wrong one to protect exactly. The floor has to survive the same
    /// treatment on a machine that has those directories and one that does not.
    #[test]
    fn widening_never_invents_an_entry_out_of_a_shape_that_cannot_be_resolved() {
        let protected = ProtectedPaths::from_env(
            &MapEnv::new(),
            Path::new("C:\\nonexistent-app-dir"),
            Path::new("C:\\nonexistent-app-dir\\q"),
        );

        for path in [
            "C:\\Windows\\System32",
            "C:\\Program Files\\App\\a.exe",
            "C:\\Program Files (x86)\\Old\\x.dll",
            "C:\\ProgramData\\Vendor\\cache.db",
            "C:\\",
            "D:\\",
            "C:\\nonexistent-app-dir\\q\\batch\\item",
            "E:\\System Volume Information\\x",
        ] {
            assert!(
                protected.is_protected(Path::new(path)),
                "`{path}` must survive both the missing directories and the widening"
            );
        }
        for path in ["C:\\Temp\\a.log", "C:\\Users\\dev\\Downloads\\x.zip"] {
            assert!(
                !protected.is_protected(Path::new(path)),
                "`{path}` must not become protected by accident"
            );
        }
    }

    /// Security review C2: the gate is an allow-list. Every spelling that is
    /// not a plain local drive-letter path is refused as unjudgeable, whatever
    /// it points at — and refused is not allowed.
    #[test]
    fn the_gate_accepts_only_plain_local_drive_letter_paths() {
        let cases: &[(&str, bool, &str)] = &[
            (
                "\\\\localhost\\C$\\Windows\\System32\\ntoskrnl.exe",
                false,
                "the administrative share reaches the local disk from a non-elevated process",
            ),
            (
                "\\\\127.0.0.1\\C$\\Windows\\System32\\ntoskrnl.exe",
                false,
                "the same thing by loopback address",
            ),
            (
                "\\\\?\\UNC\\localhost\\c$\\Windows\\System32\\ntoskrnl.exe",
                false,
                "and the canonical form the other two resolve to",
            ),
            (
                "\\??\\C:\\Windows\\System32\\x",
                false,
                "the NT object namespace",
            ),
            (
                "\\\\.\\C:\\Windows\\System32\\x",
                false,
                "the device namespace",
            ),
            (
                "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Windows\\System32\\x",
                false,
                "the volume by device name rather than drive letter",
            ),
            (
                "C:Windows\\System32\\x",
                false,
                "drive-relative: resolved against a current directory we do not control",
            ),
            (
                "\\Windows\\System32\\x",
                false,
                "rooted, but on which volume?",
            ),
            (
                "\\\\server\\share\\x",
                false,
                "a genuinely remote share is refused too, not crashed on",
            ),
            (
                "relative\\path\\x",
                false,
                "relative to nothing in particular",
            ),
            ("", false, "empty"),
            ("C:", false, "a bare drive is the drive's current directory"),
            ("C:\\Temp\\..\\Windows", false, "`..` can land anywhere"),
            (
                "C:\\Windows\\System32\\x",
                true,
                "the shape the gate exists for",
            ),
            ("c:\\temp\\a.tmp", true, "lower case is the same shape"),
            ("C:/Temp/a.tmp", true, "forward slashes are the same shape"),
            ("C:\\", true, "a drive root is a plain local path"),
            (
                "\\\\?\\C:\\Windows\\System32\\x",
                true,
                "the verbatim prefix of a plain local path is still a plain local path",
            ),
        ];

        for (path, accepted, why) in cases {
            let shape = plain_local_shape(Path::new(path));
            assert_eq!(shape.is_ok(), *accepted, "`{path}` — {why} (got {shape:?})");

            // And the verdict the callers actually see. Nothing here exists on
            // disk except `C:\` and the two real system paths, so an accepted
            // shape lands on `Local` or `Missing`; a refused one never does.
            match gate_path(Path::new(path)) {
                GateVerdict::Refused(reason) => {
                    assert!(!*accepted, "`{path}` — {why} — was refused: {reason}");
                    assert!(
                        reason.len() > 20 && reason.contains(&path.to_string()) || path.is_empty(),
                        "a refusal must name the path and explain itself, got `{reason}`"
                    );
                }
                other => assert!(*accepted, "`{path}` — {why} — got {other:?}"),
            }
        }
    }

    /// The other half of C2: a refused spelling must also be *protected* as far
    /// as the list is concerned, so a caller that only consults the list still
    /// gets the safe answer (defence in depth, C2d).
    #[test]
    fn an_unjudgeable_spelling_is_reported_as_protected_by_the_list_too() {
        let protected = windows_like();
        for path in [
            "\\\\localhost\\C$\\Windows\\System32\\ntoskrnl.exe",
            "\\\\127.0.0.1\\C$\\Windows\\System32\\ntoskrnl.exe",
            "\\\\?\\UNC\\localhost\\c$\\Windows\\System32\\ntoskrnl.exe",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Windows\\System32\\x",
            "C:Windows\\System32\\x",
            "\\Windows\\System32\\x",
            "\\\\server\\share\\x",
        ] {
            let reason = protected
                .reason_for(Path::new(path))
                .unwrap_or_else(|| panic!("`{path}` must not be reported as unprotected"));
            assert!(reason.len() > 10, "`{path}` must explain itself");
        }

        // `\??\C:\Windows` and `\\.\C:\Windows` are stripped by the lexical
        // normaliser, so the list catches them by name as well as by shape.
        for path in [
            "\\??\\C:\\Windows\\System32\\x",
            "\\\\.\\C:\\Windows\\System32\\x",
        ] {
            assert_eq!(
                split(Path::new(path))[0],
                "c:",
                "`{path}` must normalise onto the drive it really opens"
            );
            assert!(protected.is_protected(Path::new(path)));
        }
    }

    #[test]
    fn a_unc_path_never_looks_like_a_local_one() {
        let protected = windows_like();
        // Somebody else's machine — and, since C2, still refused rather than
        // waved through: this list cannot judge a share at all.
        assert!(protected.is_protected(Path::new("\\\\server\\Windows\\System32")));
        assert_eq!(
            split(Path::new("\\\\?\\UNC\\server\\share\\x")),
            vec!["\\\\server\\share", "x"],
            "the verbatim UNC spelling collapses onto the ordinary one"
        );
        assert_eq!(
            split(Path::new("\\\\server\\share\\a\\..\\..\\..\\b")),
            vec!["\\\\server\\share", "b"],
            "`..` never climbs past the share root"
        );
    }

    #[test]
    fn the_real_list_covers_every_location_the_architecture_names() {
        let protected = windows_like();
        for path in [
            "C:\\Windows",
            "C:\\Program Files",
            "C:\\Program Files (x86)",
            "C:\\Users\\dev",
            "C:\\System Volume Information",
            "C:\\",
            "C:\\Users\\dev\\AppData\\Roaming\\WinKnife\\quarantine",
        ] {
            assert!(
                protected.is_protected(Path::new(path)),
                "ARCHITECTURE section 4 requires `{path}` to be protected"
            );
        }
    }

    #[test]
    fn normalisation_is_stable() {
        assert_eq!(
            normalise(Path::new("C:/Windows/")),
            PathBuf::from("c:\\windows")
        );
        assert_eq!(normalise(Path::new("C:\\")), PathBuf::from("c:"));
        assert_eq!(
            split(Path::new("c:\\windows\\temp")),
            vec!["c:", "windows", "temp"]
        );
    }

    /// The prefix normaliser on its own, so each rule can fail on its own line.
    #[test]
    fn the_normaliser_collapses_every_windows_spelling() {
        let cases: &[(&str, &str)] = &[
            ("\\\\?\\C:\\Windows\\System32", "c:\\windows\\system32"),
            ("\\\\.\\C:\\Windows\\System32", "c:\\windows\\system32"),
            ("\\\\?\\c:\\windows", "c:\\windows"),
            ("C:\\Windows.", "c:\\windows"),
            ("C:\\Windows ", "c:\\windows"),
            ("C:\\Program Files. \\App", "c:\\program files\\app"),
            ("C:\\Temp\\..\\Windows", "c:\\windows"),
            ("C:\\..\\Windows", "c:\\windows"),
            ("C:\\Temp\\.\\sub", "c:\\temp\\sub"),
            ("C:/Temp//sub/", "c:\\temp\\sub"),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                normalise(Path::new(raw)),
                PathBuf::from(*expected),
                "normalising `{raw}`"
            );
        }
    }

    #[test]
    fn traversal_is_detected_in_either_separator() {
        assert!(has_traversal(Path::new("C:\\a\\..\\b")));
        assert!(has_traversal(Path::new("C:/a/../b")));
        assert!(!has_traversal(Path::new("C:\\a..b\\c")));
        assert!(!has_traversal(Path::new("C:\\a\\..b")));
        assert!(!has_traversal(Path::new("C:\\Windows\\System32")));
    }
}
