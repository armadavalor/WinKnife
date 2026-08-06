//! The one place in this crate where two paths are compared.
//!
//! # Why this module exists
//!
//! Windows accepts many spellings of one file, and three consecutive security
//! reviews each found the same defect in a different place: **two spellings of
//! one path compared as raw text.**
//!
//! - [`crate::core::brain::protected`] stored `C:\Users\RUNNER~1\…` while the
//!   gate asked about `C:\Users\runneradmin\…`, so a protected location quietly
//!   stopped being protected on exactly the machines whose environment is
//!   spelled that way.
//! - [`crate::core::brain::rules`] asked whether a short-form `%TEMP%` sat
//!   inside a long-form `%USERPROFILE%`, concluded it did not, and refused to
//!   load the shipped packs. A rules-load failure is fatal by design, so the
//!   application did not start at all.
//! - [`crate::core::brain::Brain::classify`] rated a path from its text while
//!   the executor re-checked the same path from its canonical form, so analyze
//!   and execute could disagree about whether one file was protected.
//!
//! Each was fixed where it was found, and the next round found the next one.
//! This module is the fix for the *class*: every path-vs-path question in the
//! crate is asked here, and the answer comes from what the disk says the two
//! paths are, not from how somebody typed them.
//!
//! # What "canonical" means here
//!
//! [`CanonicalPath::resolved`] goes through `dunce::canonicalize`, which
//! resolves 8.3 aliases, junctions, symlinks and the verbatim prefix — and
//! hands back the plain `C:\…` spelling rather than `\\?\C:\…`, so the result
//! compares against every other path the app has ever seen.
//!
//! A path that is not on disk cannot be canonicalised at all, so the longest
//! ancestor that *does* exist is canonicalised and the missing tail re-attached.
//! A tail that does not exist cannot contain a reparse point, so nothing is
//! being guessed at. `C:\` exists on every machine, which means this degrades
//! gently instead of falling off a cliff: `%TEMP%\not-created-yet` still
//! compares correctly against `%TEMP%`. When nothing on the way up resolves
//! either, the comparison falls back to the lexical normalisation — which is
//! what the whole crate did before this module existed.
//!
//! # What is deliberately *not* routed through here
//!
//! Two kinds of comparison are correctly textual, and say so at their call site:
//!
//! - **Glob matching** (`rules::CompiledRule::matches_path`). The patterns were
//!   compiled from the spelling the rule pack was written in; canonicalising the
//!   candidate would rewrite it into a spelling the pattern never mentions, and
//!   a rule would stop matching its own directory.
//! - **Shape checks** (`rules::unsafe_pattern_shape`,
//!   `protected::plain_local_shape`). These ask "how is this written?", which is
//!   a question about text by definition — and they deliberately run *before*
//!   anything is resolved, because refusing an unjudgeable spelling is the point
//!   of an allow-list gate.

use std::path::{Component, Path, PathBuf};

/// A path reduced to the single spelling a comparison is allowed to see.
///
/// Constructed one of two ways, and the choice is always a statement about cost:
///
/// - [`CanonicalPath::resolved`] asks the disk. This is the default and what
///   every safety decision uses.
/// - [`CanonicalPath::lexical`] does not. It exists for the inner loop of
///   [`crate::core::brain::ProtectedPaths::reason_for`], which compares one
///   probe against every entry in the list: resolving per entry would be a
///   syscall per entry per file examined. The entries are resolved once when the
///   list is built instead, which puts the disk lookup where it belongs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalPath {
    parts: Vec<String>,
}

impl CanonicalPath {
    /// The spelling this path really names on this disk.
    ///
    /// Canonicalises the longest ancestor that exists and re-attaches whatever
    /// tail does not, so an answer is always produced — see the module header
    /// for why that stays honest.
    pub fn resolved(path: &Path) -> Self {
        let lexical = components(path);

        // Longest first: the whole path usually exists, and then this is one
        // syscall.
        for depth in (1..=lexical.len()).rev() {
            let Ok(canonical) = dunce::canonicalize(join(&lexical[..depth])) else {
                continue;
            };
            let mut parts = components(&canonical);
            parts.extend_from_slice(&lexical[depth..]);
            return Self { parts };
        }

        Self { parts: lexical }
    }

    /// The text-only normalisation: lower-cased, backslash-separated, verbatim
    /// and NT-namespace prefixes stripped, `..` resolved lexically, the trailing
    /// dots and spaces Win32 discards trimmed.
    ///
    /// Two spellings of one directory still look different to this. Use
    /// [`CanonicalPath::resolved`] unless the call site documents why it cannot.
    pub fn lexical(path: &Path) -> Self {
        Self {
            parts: components(path),
        }
    }

    /// Whether this path is `outer` itself or sits beneath it.
    ///
    /// Whole components, so `C:\Users\dev2` is not inside `C:\Users\dev`.
    pub fn is_within(&self, outer: &Self) -> bool {
        !outer.parts.is_empty()
            && self.parts.len() >= outer.parts.len()
            && self.parts[..outer.parts.len()] == outer.parts[..]
    }

    /// The comparable components, `["c:", "windows", "system32"]`.
    pub fn parts(&self) -> &[String] {
        &self.parts
    }

    /// Whether this path reduced to nothing at all.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// A stable key for hashing, de-duplicating and grouping.
    pub fn key(&self) -> String {
        self.parts.join("\\")
    }

    /// The normalised path as a `PathBuf`. `\` when it reduced to nothing, so
    /// the result is never an empty path.
    pub fn to_path_buf(&self) -> PathBuf {
        if self.parts.is_empty() {
            return PathBuf::from("\\");
        }
        PathBuf::from(self.key())
    }
}

/// Whether `a` and `b` name the same place on this disk.
pub fn same_path(a: &Path, b: &Path) -> bool {
    CanonicalPath::resolved(a) == CanonicalPath::resolved(b)
}

/// Whether `inner` is `outer` or sits beneath it, on this disk.
///
/// This is the question `Path::starts_with` looks like it answers and does not:
/// it compares components as they were typed, so a short-form path is judged to
/// be outside its own long-form parent — and it treats `..` as an ordinary
/// component, so `C:\app\quarantine\..\..\Windows` "starts with"
/// `C:\app\quarantine`.
pub fn is_inside(outer: &Path, inner: &Path) -> bool {
    CanonicalPath::resolved(inner).is_within(&CanonicalPath::resolved(outer))
}

/// Whether `path` contains a `..` component in any spelling.
///
/// Text-based on purpose: this runs *before* anything is resolved, precisely so
/// a caller can refuse a path rather than resolve one that could land anywhere.
pub fn has_traversal(path: &Path) -> bool {
    // `Component::ParentDir` is the authoritative answer for a real `Path`, but
    // it normalises away `C:/a/../b` on non-Windows hosts, and these rules are
    // Windows semantics that have to hold identically wherever the tests run.
    path.components().any(|part| part == Component::ParentDir)
        || path
            .to_string_lossy()
            .replace('/', "\\")
            .split('\\')
            .any(|part| part == "..")
}

/// Split a path into comparable components. `C:\Windows\System32` becomes
/// `["c:", "windows", "system32"]`.
///
/// Deliberately text-based rather than `Path::components`: the normalisations
/// below (verbatim prefix, trailing dot and space, lexical `..`) are Windows
/// path semantics, and they have to apply identically whatever host the tests
/// happen to run on.
fn components(path: &Path) -> Vec<String> {
    let raw = strip_windows_prefix(&path.to_string_lossy().replace('/', "\\"));

    let mut parts: Vec<String> = Vec::new();
    // How many leading components anchor the path. `..` never climbs past the
    // anchor, matching Win32: `C:\..\Windows` is `C:\Windows`.
    let mut anchor = 0usize;
    let mut rest = raw.as_str();

    // A UNC share root is two components that only mean anything together, so
    // they are kept as one and can never be mistaken for a local directory.
    if let Some(unc) = rest.strip_prefix("\\\\") {
        let mut pieces = unc.splitn(3, '\\');
        let server = pieces.next().unwrap_or_default();
        let share = pieces.next().unwrap_or_default();
        if !server.is_empty() && !share.is_empty() {
            parts.push(format!(
                "\\\\{}\\{}",
                server.to_lowercase(),
                share.to_lowercase()
            ));
            anchor = 1;
            rest = pieces.next().unwrap_or("");
        }
    }

    for part in rest.split('\\') {
        match part {
            "" | "." => continue,
            ".." => {
                if parts.len() > anchor {
                    parts.pop();
                }
            }
            _ => {
                // Win32 discards a trailing dot when it resolves a name, so
                // `C:\Windows.\System32\ntoskrnl.exe` opens the real file.
                // A trailing space is *not* discarded — `C:\Windows \System32`
                // is NotFound on a real machine — but trimming it too is a
                // harmless over-refusal, and it costs nothing to be wrong in
                // the direction that protects more (security review L6).
                let trimmed = part.trim_end_matches(['.', ' ']).to_lowercase();
                if trimmed.is_empty() {
                    continue;
                }
                if parts.is_empty() && is_drive(&trimmed) {
                    anchor = 1;
                }
                parts.push(trimmed);
            }
        }
    }
    parts
}

/// Rebuild a path from components, in a shape `canonicalize` can open.
fn join(parts: &[String]) -> PathBuf {
    let mut text = parts.join("\\");
    // `c:` on its own is that drive's *current directory*, not its root, and
    // canonicalising it would silently resolve somewhere else entirely. Every
    // other shape is already unambiguous.
    if parts.len() == 1 && is_drive(&parts[0]) {
        text.push('\\');
    }
    PathBuf::from(text)
}

/// Remove the `\\?\` verbatim, `\\.\` device or `\??\` NT-object prefix,
/// putting a UNC path back into its ordinary `\\server\share` spelling.
///
/// All three name the same file the plain spelling does. `protected::gate_path`
/// refuses the device and NT-namespace forms outright; stripping them here keeps
/// the lexical list useful as the second layer for a caller that only has text
/// (security review C2e).
fn strip_windows_prefix(raw: &str) -> String {
    for prefix in ["\\\\?\\", "\\\\.\\", "\\??\\"] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            let unc_marker = "unc\\";
            if rest.len() >= unc_marker.len()
                && rest[..unc_marker.len()].to_lowercase() == unc_marker
            {
                return format!("\\\\{}", &rest[unc_marker.len()..]);
            }
            return rest.to_owned();
        }
    }
    raw.to_owned()
}

/// Whether a component is a drive specifier such as `c:`.
fn is_drive(part: &str) -> bool {
    let bytes = part.as_bytes();
    bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lexical_normaliser_collapses_every_windows_spelling() {
        let cases: &[(&str, &str)] = &[
            ("\\\\?\\C:\\Windows\\System32", "c:\\windows\\system32"),
            ("\\\\.\\C:\\Windows\\System32", "c:\\windows\\system32"),
            ("\\??\\C:\\Windows\\System32", "c:\\windows\\system32"),
            ("\\\\?\\c:\\windows", "c:\\windows"),
            ("C:\\Windows.", "c:\\windows"),
            ("C:\\Windows ", "c:\\windows"),
            ("C:\\Program Files. \\App", "c:\\program files\\app"),
            ("C:\\Temp\\..\\Windows", "c:\\windows"),
            ("C:\\..\\Windows", "c:\\windows"),
            ("C:\\Temp\\.\\sub", "c:\\temp\\sub"),
            ("C:/Temp//sub/", "c:\\temp\\sub"),
            ("C:/Windows/", "c:\\windows"),
            ("C:\\", "c:"),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                CanonicalPath::lexical(Path::new(raw)).key(),
                *expected,
                "normalising `{raw}`"
            );
        }
    }

    #[test]
    fn a_unc_share_root_is_one_component() {
        assert_eq!(
            CanonicalPath::lexical(Path::new("\\\\?\\UNC\\server\\share\\x")).parts(),
            ["\\\\server\\share", "x"]
        );
        assert_eq!(
            CanonicalPath::lexical(Path::new("\\\\server\\share\\a\\..\\..\\..\\b")).parts(),
            ["\\\\server\\share", "b"],
            "`..` never climbs past the share root"
        );
    }

    #[test]
    fn containment_compares_whole_components() {
        let inside = |outer: &str, inner: &str| {
            CanonicalPath::lexical(Path::new(inner))
                .is_within(&CanonicalPath::lexical(Path::new(outer)))
        };
        assert!(inside(
            "C:\\Users\\dev",
            "C:\\Users\\dev\\AppData\\Local\\Temp"
        ));
        assert!(
            inside("C:\\Users\\dev", "c:/users/DEV"),
            "a path contains itself"
        );
        assert!(!inside("C:\\Users\\dev", "C:\\Users\\dev2\\Temp"));
        assert!(!inside("C:\\Users\\dev", "C:\\Windows"));
        assert!(
            !inside("", "C:\\Windows"),
            "an empty outer must never contain anything"
        );
    }

    #[test]
    fn traversal_is_detected_in_either_separator() {
        assert!(has_traversal(Path::new("C:\\a\\..\\b")));
        assert!(has_traversal(Path::new("C:/a/../b")));
        assert!(has_traversal(Path::new("..")));
        assert!(!has_traversal(Path::new("C:\\a..b\\c")));
        assert!(!has_traversal(Path::new("C:\\a\\..b")));
        assert!(!has_traversal(Path::new("C:\\Windows\\System32")));
    }

    #[test]
    fn a_bare_drive_is_never_resolved_against_its_current_directory() {
        // `C:` means "wherever this process last was on C:", which is a real
        // path and the wrong one. Rebuilding it has to produce the root.
        assert_eq!(join(&["c:".to_owned()]), PathBuf::from("c:\\"));
        assert_eq!(
            join(&["c:".to_owned(), "windows".to_owned()]),
            PathBuf::from("c:\\windows")
        );
        assert_eq!(
            CanonicalPath::resolved(Path::new("C:\\")).key(),
            "c:",
            "a drive root resolves to itself, not to a working directory"
        );
    }

    /// The defect this module exists for, on real directories: one folder, two
    /// spellings, and a text comparison says they are different places.
    #[cfg(windows)]
    #[test]
    fn two_spellings_of_one_directory_compare_equal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let long = dir.path().join("Two Spellings Here");
        std::fs::create_dir_all(&long).expect("fixture dir");
        let canonical = dunce::canonicalize(&long).expect("canonicalize");

        let Some(short) =
            crate::core::test_support::short_path_name(&long).filter(|short| short != &canonical)
        else {
            crate::core::test_support::privileged_operation_unavailable(
                "two_spellings_of_one_directory_compare_equal",
                "no 8.3 alias is available here (fsutil 8dot3name), so the two spellings \
                 this test compares cannot be produced",
            );
            return;
        };

        assert_ne!(
            CanonicalPath::lexical(&short),
            CanonicalPath::lexical(&canonical),
            "the premise: as text these really are two different strings"
        );
        assert!(
            same_path(&short, &canonical),
            "`{}` and `{}` are one directory",
            short.display(),
            canonical.display()
        );
        assert!(
            is_inside(&canonical, &short.join("child")),
            "and containment has to see through the spelling too"
        );
        assert!(
            is_inside(&short, &canonical.join("child")),
            "in both directions: `widen` only ever added one of them"
        );
    }

    /// A path that is not on disk still has to compare correctly against one
    /// that is — the ancestor walk, which is what keeps this usable for rule
    /// roots that point at folders this machine has never created.
    #[cfg(windows)]
    #[test]
    fn a_path_that_does_not_exist_yet_resolves_through_its_parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let canonical = dunce::canonicalize(dir.path()).expect("canonicalize");
        let missing = canonical.join("not-created-yet").join("nor-this");

        assert!(
            is_inside(&canonical, &missing),
            "the tail does not exist, but the parent decides where it is"
        );
        assert!(!is_inside(&missing, &canonical));
    }
}
