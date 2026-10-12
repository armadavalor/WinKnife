//! The filesystem's own answer to "are these two paths the same file?".
//!
//! The duplicate finder needs it (module spec 03, step 5): a hard link and a
//! junction give one file two names, and two names for one file are not a
//! duplicate — offering to quarantine "the copy" would remove the only copy.
//!
//! On Windows the identity is the volume serial number plus the file index,
//! which is what `GetFileInformationByHandle` reports and what
//! `std::os::windows::fs::MetadataExt` exposes. Elsewhere it is the device and
//! inode numbers, which mean the same thing — so the module's tests run on any
//! host even though the app only ships for Windows.
//!
//! Read-only: opening a handle to ask for metadata modifies nothing.

use std::path::Path;

/// A file's identity on its volume. Equal ids mean one file, whatever it is
/// called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId {
    /// Volume serial number (Windows) or device id (elsewhere).
    pub volume: u64,
    /// File index (Windows) or inode number (elsewhere).
    pub index: u64,
}

/// The identity of the file at `path`, or `None` when the filesystem will not
/// say.
///
/// `None` is not "unique": a caller that cannot get an identity must treat the
/// file as *possibly* shared, which the duplicate finder does by falling back to
/// content comparison rather than by assuming.
pub fn file_identity(path: &Path) -> Option<FileId> {
    try_file_identity(path).ok()
}

/// The identity of the file at `path`, or why it could not be read.
///
/// For a caller that must tell "there is no file there" (`NotFound`) apart
/// from "the filesystem would not say" (anything else): the quarantine's
/// cross-volume move treats only the first as the source being gone
/// (adversarial review follow-up B).
///
/// # Errors
/// Returns the error opening `path` or reading its identity produced.
pub fn try_file_identity(path: &Path) -> std::io::Result<FileId> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };

        // `File::open`, not `symlink_metadata`: the question is which file this
        // name ends at, and for a link that is the target. The walker never
        // hands links over anyway (`core::scanner`), so this only ever sees real
        // files. Read access only — asking a handle for its metadata modifies
        // nothing, which keeps `scan` read-only.
        //
        // `std::os::windows::fs::MetadataExt::{volume_serial_number, file_index}`
        // would say the same thing in three lines, but they are still unstable
        // (`windows_by_handle`, rust-lang/rust#63010) and this crate builds on
        // stable.
        let file = std::fs::File::open(path)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the handle is owned by `file` and outlives the call, and
        // `info` is a live, correctly-sized out parameter.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| std::io::Error::last_os_error())?;

        Ok(FileId {
            volume: u64::from(info.dwVolumeSerialNumber),
            index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        })
    }

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path)?;
        Ok(FileId {
            volume: metadata.dev(),
            index: metadata.ino(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Follow-up B: a missing file says `NotFound`, so a caller can tell it
    /// apart from an identity that could not be read.
    #[test]
    fn a_missing_file_reads_as_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = try_file_identity(&dir.path().join("absent.bin")).expect_err("no file");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn one_file_has_one_identity_however_it_is_reached() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"payload").expect("write");

        let first = file_identity(&path).expect("an identity for a real file");
        let second = file_identity(&path).expect("stable across calls");
        assert_eq!(first, second);
    }

    /// The case the duplicate finder exists to get right: two names, one file.
    /// `CreateHardLinkW` needs no privilege on NTFS, so this asserts everywhere.
    #[test]
    fn a_hard_link_shares_its_target_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = dir.path().join("original.bin");
        let link = dir.path().join("hardlink.bin");
        std::fs::write(&original, b"the same bytes").expect("write");
        std::fs::hard_link(&original, &link).expect("hard links need no privilege on NTFS");

        assert_eq!(
            file_identity(&original).expect("original"),
            file_identity(&link).expect("link"),
            "a hard link is the same file, so it is not a duplicate of itself"
        );
    }

    #[test]
    fn two_separate_files_have_different_identities() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, b"identical").expect("write");
        std::fs::write(&b, b"identical").expect("write");

        assert_ne!(
            file_identity(&a).expect("a"),
            file_identity(&b).expect("b"),
            "same content is not same file"
        );
    }

    #[test]
    fn a_missing_path_has_no_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(file_identity(&dir.path().join("nope")), None);
    }
}
