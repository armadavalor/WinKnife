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
/// not misreported as inaccessible.
///
/// # Why the open does not count as modifying the file
///
/// CLAUDE.md: "All scans are read-only. `scan()` and `analyze()` must not modify
/// the filesystem." Opening a file writes none of its contents — but on a volume
/// where last-access updates are enabled it moves the file's **last-access
/// timestamp**, and that timestamp is a signal the scoring pipeline reads
/// (`brain::score::Signals::idle_days`). So the scan mutated the very thing it
/// was measuring: the first scan of the day bumped every candidate it examined,
/// and the second found nothing old enough to be safe (architect item A1).
///
/// Windows documents a suppression for exactly this. Passing a `FILETIME` of
/// `{0xFFFFFFFF, 0xFFFFFFFF}` as `SetFileTime`'s last-access argument disables
/// access-time updates for the remaining lifetime of that handle, so the probe
/// leaves the clock where it found it. [`suppress_access_time_updates`] applies
/// it immediately after the open, before anything can touch the file.
///
/// The two-clock reading in `brain::score` stays as defence in depth: this fix
/// removes the cause, that one survives a probe we did not write.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLockProbe;

impl LockProbe for SystemLockProbe {
    fn probe(&self, path: &Path) -> LockState {
        #[cfg(windows)]
        {
            // A directory cannot be opened as a file, so the exclusive-open
            // question cannot be asked about one. It used to be answered
            // "free" anyway, which is a claim this probe has no evidence for:
            // any of a thousand children could be held open, and the caller
            // treats Free as permission to move (security review M9). The
            // honest answer is that we cannot tell.
            if path.is_dir() {
                return LockState::Inaccessible;
            }

            match open_leaving_the_clock_alone(path) {
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

/// Open `path` exclusively without letting the open move its last-access time.
///
/// Two attempts, and the second is the reason this is a function rather than a
/// line:
///
/// 1. `FILE_GENERIC_READ | FILE_WRITE_ATTRIBUTES`. The extra right is what
///    `SetFileTime` needs, and it is the only way to ask for the suppression.
/// 2. `FILE_GENERIC_READ` alone, if the first was refused. An ACL that grants
///    read but not write-attributes is unusual and entirely legal, and a probe
///    that reported such a file `Inaccessible` would quietly stop the cleaner
///    offering anything in that tree. Losing the suppression is the smaller
///    harm — and `brain::score` already reads two clocks precisely so a bumped
///    access time cannot make a fresh file look forgotten.
///
/// The error from the *second* attempt is the one returned, because it is the
/// plain-read question the caller is actually asking: it classifies a sharing
/// violation as `InUse` and a denial as `Inaccessible` exactly as before.
#[cfg(windows)]
fn open_leaving_the_clock_alone(path: &Path) -> std::io::Result<std::fs::File> {
    use windows::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_WRITE_ATTRIBUTES};

    if let Ok(file) = open_exclusive(path, FILE_GENERIC_READ.0 | FILE_WRITE_ATTRIBUTES.0) {
        if suppress_access_time_updates(&file) {
            return Ok(file);
        }
        // The handle is good but the suppression was refused; the fall-through
        // would only re-open it for the same answer, so keep this one.
        tracing::debug!(
            path = %path.display(),
            "could not suppress last-access updates for this handle"
        );
        return Ok(file);
    }

    open_exclusive(path, FILE_GENERIC_READ.0)
}

/// One exclusive open with an explicit desired-access mask.
#[cfg(windows)]
fn open_exclusive(path: &Path, access: u32) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .access_mode(access)
        .share_mode(0) // no FILE_SHARE_* flags: exclusive.
        .open(path)
}

/// Turn off last-access-time updates for `file`'s handle. Returns whether the
/// call was accepted.
///
/// The magic value is Windows' documented opt-out: a `FILETIME` whose halves are
/// both `0xFFFFFFFF` means "make no further access-time updates through this
/// handle" rather than "set the timestamp to that value". The creation and
/// last-write arguments are `None`, so nothing else about the file is touched —
/// which is what keeps `scan()` read-only.
#[cfg(windows)]
fn suppress_access_time_updates(file: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{FILETIME, HANDLE};
    use windows::Win32::Storage::FileSystem::SetFileTime;

    const LEAVE_ACCESS_TIME_ALONE: FILETIME = FILETIME {
        dwLowDateTime: 0xFFFF_FFFF,
        dwHighDateTime: 0xFFFF_FFFF,
    };

    let handle = HANDLE(file.as_raw_handle());
    // SAFETY: `handle` is borrowed from `file`, which outlives the call, and the
    // only pointer passed is to a `const` that lives for the whole program.
    unsafe { SetFileTime(handle, None, Some(&LEAVE_ACCESS_TIME_ALONE), None) }.is_ok()
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

    /// Security review M9: a directory is never reported free, because this
    /// probe has no way to prove it.
    #[test]
    fn a_directory_is_never_reported_free() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("sub")).expect("sub");
        std::fs::write(dir.path().join("sub/held.txt"), b"x").expect("write");

        let state = SystemLockProbe.probe(&dir.path().join("sub"));
        assert!(
            !state.is_free(),
            "a directory must not be reported as safe to move"
        );
        assert!(!state.reason().is_empty());
    }

    /// **Architect item A1**: a scan must not modify the filesystem, and the
    /// last-access timestamp is part of the filesystem — it is also a signal the
    /// scoring pipeline reads back, so a probe that bumped it made the second
    /// scan of the day find nothing safe.
    ///
    /// Backdated first, so the assertion is on an exact instant rather than on
    /// "close enough to now", and so it holds whatever the volume's last-access
    /// policy is. How much it proves does depend on that policy, and the test
    /// says which regime it ran in rather than leaving the reader to assume:
    /// where an unsuppressed open already moves nothing, this passes for a
    /// second reason. `a_suppressed_handle_keeps_the_clock_even_when_read` is
    /// the one that pins the mechanism itself.
    #[cfg(windows)]
    #[test]
    fn probing_a_file_leaves_its_last_access_time_alone() {
        use std::time::{Duration, SystemTime};
        use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("atime.bin");
        std::fs::write(&path, b"payload").expect("write");

        let accessed = |p: &std::path::Path| {
            std::fs::metadata(p)
                .expect("metadata")
                .accessed()
                .expect("an access time")
        };

        // A month ago: any update at all is unmissable.
        let backdated = SystemTime::now() - Duration::from_secs(60 * 60 * 24 * 30);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open for set_times")
            .set_times(std::fs::FileTimes::new().set_accessed(backdated))
            .expect("backdate the access time");

        let before = accessed(&path);

        // The control **is the old code path**: the same exclusive open with no
        // suppression on it. Running both in one test means the output shows the
        // difference the fix makes rather than only that the fix holds — and it
        // says out loud whether this volume updates access times at all, because
        // where it does not, the assertion below is true for a second reason.
        drop(open_exclusive(&path, FILE_GENERIC_READ.0).expect("the unsuppressed open"));
        let after_unsuppressed = accessed(&path);
        let updates_are_live = after_unsuppressed != before;
        println!(
            "last-access updates on this volume: {}",
            if updates_are_live {
                "ENABLED — an unsuppressed open moves the clock, so this test is load-bearing"
            } else {
                "disabled — nothing here could move the clock either way"
            }
        );
        println!("  backdated to        {before:?}");
        println!("  after plain open    {after_unsuppressed:?}  <- what the probe used to do");

        // Re-backdate, so the control open above is not what the probe is
        // measured against.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open for set_times")
            .set_times(std::fs::FileTimes::new().set_accessed(backdated))
            .expect("backdate the access time again");
        let before_probe = accessed(&path);

        assert_eq!(SystemLockProbe.probe(&path), LockState::Free);
        let after_probe = accessed(&path);
        println!("  after the probe     {after_probe:?}  <- what it does now");

        assert_eq!(
            after_probe, before_probe,
            "the probe moved the last-access time, so `scan()` is not read-only"
        );
    }

    /// The mechanism itself, pinned against a control that provably *can* move
    /// the clock on the machine the test is running on.
    ///
    /// A bare open is too weak a control: this development machine reports
    /// `DisableLastAccess = 2 (System Managed, Disabled)`, where opening a file
    /// moves nothing — so `probing_a_file_leaves_its_last_access_time_alone`
    /// would pass there even with the `SetFileTime` call deleted. Reading the
    /// file's **contents** does move it even under that policy, which gives a
    /// control with teeth:
    ///
    /// ```text
    /// backdate -> read through a plain handle       -> clock MOVES
    /// backdate -> read through a suppressed handle  -> clock HOLDS
    /// ```
    ///
    /// If the first line does not move on some volume, the comparison is
    /// vacuous there and the test says so instead of claiming a result; the
    /// `SetFileTime` call is still asserted to have been accepted.
    #[cfg(windows)]
    #[test]
    fn a_suppressed_handle_keeps_the_clock_even_when_read() {
        use std::io::Read;
        use std::time::{Duration, SystemTime};
        use windows::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_WRITE_ATTRIBUTES};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("suppress.bin");
        std::fs::write(&path, vec![7u8; 64 * 1024]).expect("write");

        let accessed = |p: &std::path::Path| {
            std::fs::metadata(p)
                .expect("metadata")
                .accessed()
                .expect("an access time")
        };
        let backdated = SystemTime::now() - Duration::from_secs(60 * 60 * 24 * 30);
        let backdate = |p: &std::path::Path| {
            std::fs::OpenOptions::new()
                .write(true)
                .open(p)
                .expect("open for set_times")
                .set_times(std::fs::FileTimes::new().set_accessed(backdated))
                .expect("backdate the access time");
        };
        let drain = |file: &mut std::fs::File| {
            let mut sink = Vec::new();
            file.read_to_end(&mut sink).expect("read the contents");
        };

        // Control: an unsuppressed handle, read to the end.
        backdate(&path);
        let before_control = accessed(&path);
        let mut plain = open_exclusive(&path, FILE_GENERIC_READ.0).expect("plain read open");
        drain(&mut plain);
        drop(plain);
        let after_control = accessed(&path);

        // Subject: the same read through a handle carrying the suppression.
        backdate(&path);
        let before_subject = accessed(&path);
        let mut suppressed = open_exclusive(&path, FILE_GENERIC_READ.0 | FILE_WRITE_ATTRIBUTES.0)
            .expect("an ordinary file in a tempdir grants write-attributes");
        assert!(
            suppress_access_time_updates(&suppressed),
            "SetFileTime rejected the documented last-access suppression"
        );
        drain(&mut suppressed);
        drop(suppressed);
        let after_subject = accessed(&path);

        println!("  plain handle,      read: {before_control:?} -> {after_control:?}");
        println!("  suppressed handle, read: {before_subject:?} -> {after_subject:?}");

        if after_control == before_control {
            println!(
                "  NOTE: this volume did not move the clock even for the unsuppressed \
                 control, so the comparison proves nothing here beyond SetFileTime \
                 being accepted."
            );
            return;
        }

        println!("  control moved the clock, so the suppression below is load-bearing");
        assert_eq!(
            after_subject, before_subject,
            "the suppression did not hold: a read through the handle still moved the clock"
        );
    }

    /// The fallback path: a handle without `FILE_WRITE_ATTRIBUTES` cannot carry
    /// the suppression, and the probe must still answer rather than reporting a
    /// perfectly readable file as inaccessible.
    #[cfg(windows)]
    #[test]
    fn a_read_only_open_still_answers_the_lock_question() {
        use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plain.bin");
        std::fs::write(&path, b"payload").expect("write");

        let file = open_exclusive(&path, FILE_GENERIC_READ.0).expect("plain read open");
        assert!(
            !suppress_access_time_updates(&file),
            "a handle with no write-attributes right must not claim the suppression worked"
        );
        drop(file);

        assert_eq!(SystemLockProbe.probe(&path), LockState::Free);
    }

    #[test]
    fn the_always_unlocked_stub_never_blocks() {
        assert_eq!(
            AlwaysUnlocked.probe(Path::new("C:\\does\\not\\exist")),
            LockState::Free
        );
    }
}
