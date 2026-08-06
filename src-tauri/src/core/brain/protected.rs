//! Paths that can never be rated Safe and never receive a delete action.
//!
//! Why this is code and not rule data: a rule pack is community-editable JSON.
//! If the protected list lived there, a well-meaning pull request could hand
//! someone a green "clean" button pointed at `C:\Windows`. The list is compiled
//! in, checked before any rule is consulted, and re-checked by the executor
//! (CLAUDE.md hard rules, ARCHITECTURE section 4).
//!
//! Two scopes, and the difference matters:
//!
//! - [`Scope::Tree`] — the directory and everything under it. `C:\Windows`,
//!   `C:\Program Files`, our own quarantine.
//! - [`Scope::Exact`] — the directory itself but not its children. The user
//!   profile root (`C:\Users\dev` is protected, `C:\Users\dev\Downloads` is
//!   fair game) and drive roots, which would otherwise protect everything.

use std::path::{Component, Path, PathBuf};

use crate::core::platform::EnvResolver;

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
}

impl ProtectedPaths {
    /// An empty list. Only for tests that are about something else — the real
    /// app always builds one from [`ProtectedPaths::from_env`].
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from explicit entries.
    pub fn new(entries: Vec<ProtectedEntry>) -> Self {
        Self { entries }
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
    /// `quarantine_root` and `app_data_dir` are ours: cleaning our own storage
    /// would let the app eat the very files it promised to keep restorable.
    pub fn from_env(env: &dyn EnvResolver, app_data_dir: &Path, quarantine_root: &Path) -> Self {
        let mut entries = Vec::new();

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

        tree(
            env.var("WINDIR").or_else(|| env.var("SystemRoot")),
            "the Windows directory: removing anything here can stop the system booting",
        );
        tree(
            env.var("ProgramFiles"),
            "an installed-programs directory: use the app's own uninstaller instead",
        );
        tree(
            env.var("ProgramFiles(x86)"),
            "an installed-programs directory: use the app's own uninstaller instead",
        );
        tree(
            env.var("ProgramW6432"),
            "an installed-programs directory: use the app's own uninstaller instead",
        );
        tree(
            env.var("ProgramData"),
            "shared application data used by installed programs",
        );

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
    /// known).
    pub fn push(&mut self, entry: ProtectedEntry) {
        self.entries.push(entry);
    }

    /// Every entry, for tests and diagnostics.
    pub fn entries(&self) -> &[ProtectedEntry] {
        &self.entries
    }

    /// Why `path` is protected, or `None` when it is not.
    ///
    /// Comparison is case-insensitive and separator-agnostic, matching Windows,
    /// and works on paths that do not exist so the rules can be tested without
    /// a filesystem.
    pub fn reason_for(&self, path: &Path) -> Option<&str> {
        let candidate = normalise(path);
        let candidate_components: Vec<_> = split(&candidate);

        for entry in &self.entries {
            let entry_components: Vec<_> = split(&entry.path);
            if entry_components.is_empty() {
                continue;
            }

            let matched = match entry.scope {
                Scope::Exact => candidate_components == entry_components,
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

/// Lower-case, forward slashes turned into backslashes, trailing separators
/// removed. Not `canonicalize`: that needs the path to exist, and the brain has
/// to be able to judge a path it has only been told about.
fn normalise(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy().replace('/', "\\");
    let trimmed = raw.trim_end_matches('\\');
    PathBuf::from(if trimmed.is_empty() { "\\" } else { trimmed }.to_lowercase())
}

/// Split a normalised path into comparable components. `C:\Windows\System32`
/// becomes `["c:", "windows", "system32"]`.
fn split(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => Some(
                prefix
                    .as_os_str()
                    .to_string_lossy()
                    .trim_end_matches('\\')
                    .to_lowercase(),
            ),
            Component::Normal(part) => Some(part.to_string_lossy().to_lowercase()),
            // A leading separator carries no identity of its own; the drive
            // prefix already anchors the path.
            Component::RootDir | Component::CurDir => None,
            Component::ParentDir => Some("..".to_owned()),
        })
        .filter(|part| !part.is_empty())
        .collect()
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
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\Cleanify"),
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\Cleanify\\quarantine"),
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
                "C:\\Users\\dev\\AppData\\Roaming\\Cleanify\\cleanify.sqlite3",
                true,
                "our own data",
            ),
            (
                "C:\\Users\\dev\\AppData\\Roaming\\Cleanify\\quarantine\\x\\y",
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

    #[test]
    fn missing_environment_variables_are_simply_skipped() {
        let protected = ProtectedPaths::from_env(
            &MapEnv::new(),
            Path::new("C:\\app"),
            Path::new("C:\\app\\q"),
        );
        // No WINDIR in this environment, so nothing claims it — but drive roots
        // and our own directories are unconditional.
        assert!(!protected.is_protected(Path::new("C:\\Windows\\System32")));
        assert!(protected.is_protected(Path::new("C:\\")));
        assert!(protected.is_protected(Path::new("C:\\app\\q\\batch\\item")));
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
            "C:\\Users\\dev\\AppData\\Roaming\\Cleanify\\quarantine",
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
}
