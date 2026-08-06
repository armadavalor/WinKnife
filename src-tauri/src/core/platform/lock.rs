//! "Is this file in use?" probe.
//!
//! Why it matters twice: the brain uses it as a hard gate (a file another
//! process is holding open can never be rated Safe), and the executor uses it
//! again at execute time, because a file can be opened in the seconds between
//! the review screen and the confirm button.
//!
//! The probe is deliberately conservative: anything it cannot open exclusively
//! counts as in use. A false "in use" costs the user nothing (the item stays);
//! a false "free" could take a file out from under a running program.

use std::path::Path;

/// What a probe concluded about a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    /// Nothing else holds the file open and we may act on it.
    Free,
    /// Another process holds it open.
    InUse,
    /// We are not allowed to open it at all (permissions, ACLs).
    Inaccessible,
}

impl LockState {
    /// Whether the file may be moved.
    pub fn is_free(self) -> bool {
        matches!(self, Self::Free)
    }

    /// User-facing explanation, used verbatim in recommendations and reports.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Free => "not in use",
            Self::InUse => "in use by a running program",
            Self::Inaccessible => "not accessible with the current permissions",
        }
    }
}

/// Checks whether a path is currently held open by something else.
pub trait LockProbe: Send + Sync {
    /// Inspect `path`. Must not modify it.
    fn probe(&self, path: &Path) -> LockState;
}

/// The real probe: try to open the file for reading with no sharing allowed.
///
/// On Windows an exclusive open fails while any other handle exists, which is
/// exactly the question being asked. Read access only, so a read-only file is
/// not misreported as inaccessible. Opening a file never modifies it, so this
/// is safe to call from the read-only `analyze` path.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLockProbe;

impl LockProbe for SystemLockProbe {
    fn probe(&self, path: &Path) -> LockState {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            // A directory cannot be opened as a file; treat it as free and let
            // the move itself fail if something inside is held.
            if path.is_dir() {
                return LockState::Free;
            }

            let opened = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0) // no FILE_SHARE_* flags: exclusive.
                .open(path);

            match opened {
                Ok(_) => LockState::Free,
                Err(err) => match err.kind() {
                    std::io::ErrorKind::PermissionDenied => LockState::Inaccessible,
                    // ERROR_SHARING_VIOLATION (32) and ERROR_LOCK_VIOLATION (33)
                    // both mean "someone else has it".
                    _ => match err.raw_os_error() {
                        Some(32) | Some(33) => LockState::InUse,
                        _ => LockState::Inaccessible,
                    },
                },
            }
        }

        #[cfg(not(windows))]
        {
            // Advisory locking only elsewhere; report what we can prove.
            match std::fs::OpenOptions::new().read(true).open(path) {
                Ok(_) => LockState::Free,
                Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                    LockState::Inaccessible
                }
                Err(_) => LockState::Inaccessible,
            }
        }
    }
}

/// A probe that reports everything as free. For tests that are not about
/// locking, so they do not depend on the host filesystem's mood.
#[derive(Debug, Clone, Copy, Default)]
pub struct AlwaysUnlocked;

impl LockProbe for AlwaysUnlocked {
    fn probe(&self, _path: &Path) -> LockState {
        LockState::Free
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_states_explain_themselves() {
        assert!(LockState::Free.is_free());
        assert!(!LockState::InUse.is_free());
        assert!(!LockState::Inaccessible.is_free());
        for state in [LockState::Free, LockState::InUse, LockState::Inaccessible] {
            assert!(!state.reason().is_empty());
        }
    }

    #[test]
    fn an_idle_file_in_a_tempdir_is_free() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("idle.txt");
        std::fs::write(&path, b"hello").expect("write");

        assert_eq!(SystemLockProbe.probe(&path), LockState::Free);
    }

    #[test]
    fn a_missing_file_is_not_reported_as_free() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = SystemLockProbe.probe(&dir.path().join("nope.txt"));
        assert!(!state.is_free(), "a missing path must never look free");
    }

    /// The whole point of the probe: hold a real handle open with no sharing
    /// and confirm the probe notices.
    #[cfg(windows)]
    #[test]
    fn a_file_held_open_exclusively_is_reported_in_use() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("locked.bin");
        std::fs::write(&path, b"payload").expect("write");

        let _held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("hold the file open exclusively");

        assert_eq!(SystemLockProbe.probe(&path), LockState::InUse);
    }

    #[test]
    fn the_always_unlocked_stub_never_blocks() {
        assert_eq!(
            AlwaysUnlocked.probe(Path::new("C:\\does\\not\\exist")),
            LockState::Free
        );
    }
}
