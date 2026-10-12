//! Handles that hold a file in place while quarantine moves it to another
//! volume.
//!
//! Why (security review C): a cross-volume move copies a file, proves the copy
//! and only then removes the original. Between the proof and the removal
//! another program could write to the original, or swap a different file in
//! under its name, and the removal would destroy bytes that were never copied.
//! A handle opened with read and delete access that shares only reading closes
//! that window: while it is held no other program can write, rename or delete
//! the file, and a program that already has it open for writing makes the open
//! fail instead.
//!
//! This module only opens and inspects. Removing anything stays in
//! `core::quarantine`, the one module allowed to remove user data.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use super::FileId;

/// Open `path` so that nothing else can change, rename or delete it while the
/// returned handle lives.
///
/// Fails with a sharing violation (raw os error 32) when another program
/// already has the file open for writing or deleting. A link is opened as
/// itself rather than followed; the caller checks [`is_reparse_point`].
#[cfg(windows)]
// Flags are disallowed crate-wide (clippy.toml): this one only opens a link
// as itself, never FILE_FLAG_DELETE_ON_CLOSE.
#[allow(clippy::disallowed_methods)]
pub(crate) fn open_pinned(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    };

    std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ.0 | DELETE.0)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

/// Elsewhere there is no share mode that locks other programs out; the caller
/// re-checks the file's identity before removing it instead.
#[cfg(not(windows))]
pub(crate) fn open_pinned(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Open `path` only to check which file it is and then remove it through the
/// same handle, so the check and the removal cannot be about two different
/// files.
///
/// Shares everything: it is for cleaning up a file this app created, and that
/// must not fail just because something else is reading it.
///
/// Only `core::quarantine`, the one module allowed to remove user data, may
/// call it (adversarial review, LOW 4). Two things hold it there: the
/// compiler limits callers to `crate::core` (`pub(in crate::core)`), and the
/// guard test `quarantine::tests::no_other_module_removes_user_data` limits
/// them to `core/quarantine.rs`.
#[cfg(windows)]
// Flags are disallowed crate-wide (clippy.toml): this one only opens a link
// as itself, never FILE_FLAG_DELETE_ON_CLOSE.
#[allow(clippy::disallowed_methods)]
pub(in crate::core) fn open_to_delete(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    std::fs::OpenOptions::new()
        .access_mode(DELETE.0 | FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

/// Whether the open file is a reparse point: a link, or a placeholder (cloud,
/// deduplicated, system-compressed) whose bytes are not plainly on this volume.
#[cfg(windows)]
pub(crate) fn is_reparse_point(file: &File) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    Ok((file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0)
}

/// Links are never opened through a handle elsewhere (`File::open` follows
/// them, and the walk refuses them before that), so there is nothing to report.
#[cfg(not(windows))]
pub(crate) fn is_reparse_point(_file: &File) -> io::Result<bool> {
    Ok(false)
}

/// The identity of the open file: the same [`FileId`] that
/// [`super::file_identity`] reports for its path, read from the handle itself
/// so it cannot be about a different file swapped in under the name since.
#[cfg(windows)]
pub(crate) fn handle_identity(file: &File) -> io::Result<FileId> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is owned by `file` and outlives the call, and `info`
    // is a live, correctly-sized out parameter.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|_| io::Error::last_os_error())?;

    Ok(FileId {
        volume: u64::from(info.dwVolumeSerialNumber),
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

/// Device and inode numbers, which mean the same thing as the Windows pair.
#[cfg(not(windows))]
pub(crate) fn handle_identity(file: &File) -> io::Result<FileId> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(FileId {
        volume: metadata.dev(),
        index: metadata.ino(),
    })
}

/// What the filesystem says about an open file beyond its metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StandardInfo {
    /// How many names the file has: more than one means hard links.
    pub(crate) links: u32,
    /// Whether a delete is marked and will take effect when the last handle
    /// closes.
    pub(crate) delete_pending: bool,
}

/// The link count and delete-pending state of the open file, read through the
/// handle itself.
///
/// Why (adversarial review, LOWs 1 and 3): taking back a marked delete can
/// report success and still leave it pending, so the take-back is confirmed
/// here; and a file with a second name cannot be pinned twice, so it is
/// recognised here rather than reported as open in another program.
#[cfg(windows)]
pub(crate) fn standard_info(file: &File) -> io::Result<StandardInfo> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileStandardInfo, GetFileInformationByHandleEx, FILE_STANDARD_INFO,
    };

    let mut info = FILE_STANDARD_INFO::default();
    let size = u32::try_from(std::mem::size_of::<FILE_STANDARD_INFO>())
        .map_err(|_| io::Error::other("FILE_STANDARD_INFO does not fit a u32"))?;
    // SAFETY: the handle is owned by `file` and outlives the call, and `info`
    // is a live, writable FILE_STANDARD_INFO whose exact size is passed.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileStandardInfo,
            std::ptr::from_mut(&mut info).cast(),
            size,
        )
    }
    .map_err(|_| io::Error::last_os_error())?;

    Ok(StandardInfo {
        links: info.NumberOfLinks,
        delete_pending: info.DeletePending,
    })
}

/// The link count from the metadata; nothing is ever marked for deletion
/// through a handle elsewhere.
#[cfg(not(windows))]
pub(crate) fn standard_info(file: &File) -> io::Result<StandardInfo> {
    use std::os::unix::fs::MetadataExt;

    let links = u32::try_from(file.metadata()?.nlink()).unwrap_or(u32::MAX);
    Ok(StandardInfo {
        links,
        delete_pending: false,
    })
}

/// Whether the file is encrypted with Windows EFS. A copy to another volume
/// is written in the clear, so the cross-volume move refuses one (adversarial
/// review, LOW 2).
#[cfg(windows)]
pub(crate) fn is_encrypted(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_ENCRYPTED;

    metadata.file_attributes() & FILE_ATTRIBUTE_ENCRYPTED.0 != 0
}

/// There is no EFS elsewhere.
#[cfg(not(windows))]
pub(crate) fn is_encrypted(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Open the directory at `path` itself, never what a link there points to,
/// so that nothing else can rename or delete it while the returned handle
/// lives. Files inside can still be created and changed; each of those is
/// pinned on its own.
#[cfg(windows)]
// Flags are disallowed crate-wide (clippy.toml): these only open a directory,
// and a link as itself, never FILE_FLAG_DELETE_ON_CLOSE.
#[allow(clippy::disallowed_methods)]
pub(crate) fn open_pinned_dir(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };

    std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ.0 | DELETE.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

/// Elsewhere a directory opens like a file; the walk refuses links before
/// it gets here.
#[cfg(not(windows))]
pub(crate) fn open_pinned_dir(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Open a directory, never what a link there points to, only to check it and
/// remove it through the same handle. Shares everything, like
/// [`open_to_delete`], and like it only for `core::quarantine`: the compiler
/// limits callers to `crate::core` (`pub(in crate::core)`), and the guard test
/// `quarantine::tests::no_other_module_removes_user_data` limits them to
/// `core/quarantine.rs`.
#[cfg(windows)]
// Flags are disallowed crate-wide (clippy.toml): these only open a directory,
// and a link as itself, never FILE_FLAG_DELETE_ON_CLOSE.
#[allow(clippy::disallowed_methods)]
pub(in crate::core) fn open_dir_to_delete(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    std::fs::OpenOptions::new()
        .access_mode(DELETE.0 | FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

/// Open one named stream of a file (`C:\a.txt:Zone.Identifier`) for reading,
/// so that nothing else can write to it while the returned handle lives.
///
/// Delete is shared, because a stream opened without it stops the whole file
/// being deleted, the pinned main stream included. Removing or renaming the
/// stream meanwhile changes the stream listing, which the move checks again
/// once the delete is marked.
#[cfg(windows)]
pub(crate) fn open_pinned_stream(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_DELETE.0)
        .open(path)
}

/// Elsewhere there are no named streams to open.
#[cfg(not(windows))]
pub(crate) fn open_pinned_stream(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// One data stream of an open file: `::$DATA` is the main stream and
/// `:name:$DATA` a named one, such as the `:Zone.Identifier:$DATA` that marks
/// a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataStream {
    pub(crate) name: String,
    pub(crate) size: u64,
}

/// The named data streams of the open file, sorted by name: everything but
/// the main stream, which the file's length and bytes already describe. None
/// for a directory without streams, nor on a filesystem that has no named
/// streams at all (FAT, exFAT).
///
/// Why (cross-volume review, item 2): a copy that reads the main stream
/// leaves every named stream behind, and deleting the original then destroys
/// them. A caller lists them first so it can carry the ones it knows and
/// refuse the rest.
#[cfg(windows)]
pub(crate) fn named_streams(file: &File) -> io::Result<Vec<DataStream>> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{FileStreamInfo, GetFileInformationByHandleEx};
    // What the call answers for no streams at all, and for a buffer too small.
    const ERROR_HANDLE_EOF: i32 = 38;
    const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
    const ERROR_MORE_DATA: i32 = 234;
    // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED and ERROR_INVALID_PARAMETER:
    // what a filesystem without named streams answers.
    const NO_STREAMS: [i32; 3] = [1, 50, 87];
    // 4 KiB to start, doubling to 1 MiB: far beyond any real file's streams.
    const FIRST_WORDS: usize = 512;
    const MAX_WORDS: usize = 128 * 1024;

    // `u64` words, so the buffer is aligned for FILE_STREAM_INFO.
    let mut words = vec![0u64; FIRST_WORDS];
    loop {
        let bytes = u32::try_from(words.len() * std::mem::size_of::<u64>())
            .map_err(|_| io::Error::other("stream listing buffer too large"))?;
        // SAFETY: the handle is owned by `file` and outlives the call, and
        // `words` is a live, writable buffer of exactly `bytes` bytes.
        let listed = unsafe {
            GetFileInformationByHandleEx(
                HANDLE(file.as_raw_handle()),
                FileStreamInfo,
                words.as_mut_ptr().cast(),
                bytes,
            )
        }
        .map_err(|_| io::Error::last_os_error());
        match listed {
            Ok(()) => {
                let raw: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
                return parse_streams(&raw);
            }
            Err(err)
                if err
                    .raw_os_error()
                    .is_some_and(|code| code == ERROR_HANDLE_EOF || NO_STREAMS.contains(&code)) =>
            {
                return Ok(Vec::new());
            }
            Err(err)
                if matches!(
                    err.raw_os_error(),
                    Some(ERROR_MORE_DATA | ERROR_INSUFFICIENT_BUFFER)
                ) && words.len() < MAX_WORDS =>
            {
                words.resize(words.len() * 2, 0);
            }
            Err(err) => return Err(err),
        }
    }
}

/// Elsewhere a file has one stream of data and nothing to list.
#[cfg(not(windows))]
pub(crate) fn named_streams(_file: &File) -> io::Result<Vec<DataStream>> {
    Ok(Vec::new())
}

/// Read the chain of FILE_STREAM_INFO records, failing on anything that does
/// not fit rather than guessing.
#[cfg(windows)]
fn parse_streams(raw: &[u8]) -> io::Result<Vec<DataStream>> {
    // NextEntryOffset, StreamNameLength, StreamSize, StreamAllocationSize.
    const HEADER: usize = 24;
    const MAIN_STREAM: &str = "::$DATA";
    let malformed = || io::Error::new(io::ErrorKind::InvalidData, "malformed stream listing");
    let field = |at: usize, len: usize| -> io::Result<&[u8]> {
        at.checked_add(len)
            .and_then(|end| raw.get(at..end))
            .ok_or_else(malformed)
    };
    let word = |at: usize| -> io::Result<usize> {
        let bytes: [u8; 4] = field(at, 4)?.try_into().map_err(|_| malformed())?;
        usize::try_from(u32::from_le_bytes(bytes)).map_err(|_| malformed())
    };

    let mut streams = Vec::new();
    let mut offset = 0usize;
    loop {
        let next = word(offset)?;
        let name_bytes = word(offset + 4)?;
        let size: [u8; 8] = field(offset + 8, 8)?.try_into().map_err(|_| malformed())?;
        let name = field(offset + HEADER, name_bytes)?;
        if name.len() % 2 != 0 {
            return Err(malformed());
        }
        let units: Vec<u16> = name
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .collect();
        let name = String::from_utf16(&units).map_err(|_| malformed())?;
        if !name.eq_ignore_ascii_case(MAIN_STREAM) {
            streams.push(DataStream {
                name,
                size: u64::try_from(i64::from_le_bytes(size)).map_err(|_| malformed())?,
            });
        }
        if next == 0 {
            break;
        }
        offset = offset.checked_add(next).ok_or_else(malformed)?;
    }
    streams.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(streams)
}

/// Where the open file really is, as the volume reports it now, in the
/// `\\?\C:\...` spelling: links and short names already resolved.
#[cfg(windows)]
pub(crate) fn final_path(file: &File) -> io::Result<Option<PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, GETFINALPATHNAMEBYHANDLE_FLAGS,
        VOLUME_NAME_DOS,
    };
    // The longest path Windows has, plus its NUL.
    const LONGEST: usize = 32_768;
    const FIRST_UNITS: usize = 512;

    let flags = GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0);
    let mut buffer = vec![0u16; FIRST_UNITS];
    loop {
        // SAFETY: the handle is owned by `file` and outlives the call, and
        // the slice is a live, writable buffer.
        let len =
            unsafe { GetFinalPathNameByHandleW(HANDLE(file.as_raw_handle()), &mut buffer, flags) }
                as usize;
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        if len < buffer.len() {
            let path = std::ffi::OsString::from_wide(&buffer[..len]);
            return Ok(Some(PathBuf::from(path)));
        }
        if len > LONGEST {
            return Err(io::Error::other("final path longer than any Windows path"));
        }
        // Too small: `len` is the size needed, NUL included.
        buffer.resize(len, 0);
    }
}

/// Elsewhere a handle cannot say where it is.
#[cfg(not(windows))]
pub(crate) fn final_path(_file: &File) -> io::Result<Option<PathBuf>> {
    Ok(None)
}

/// The attributes a move keeps: read-only, hidden, system, archive and
/// not-content-indexed. Compression, encryption, sparseness and the like
/// belong to how a volume stores a file, not to the file.
#[cfg(windows)]
pub(crate) const KEPT_ATTRIBUTES: u32 = {
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NOT_CONTENT_INDEXED,
        FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
    };
    FILE_ATTRIBUTE_READONLY.0
        | FILE_ATTRIBUTE_HIDDEN.0
        | FILE_ATTRIBUTE_SYSTEM.0
        | FILE_ATTRIBUTE_ARCHIVE.0
        | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED.0
};

/// The kept attributes of a file or directory (see [`KEPT_ATTRIBUTES`]).
#[cfg(windows)]
pub(crate) fn kept_attributes(metadata: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & KEPT_ATTRIBUTES
}

/// Elsewhere the read-only flag, the one attribute there is, stands in.
#[cfg(not(windows))]
pub(crate) fn kept_attributes(metadata: &std::fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

/// Open a directory as itself, to read and write its attributes.
#[cfg(windows)]
// Flags are disallowed crate-wide (clippy.toml): these only open a directory,
// and a link as itself, never FILE_FLAG_DELETE_ON_CLOSE.
#[allow(clippy::disallowed_methods)]
pub(crate) fn open_dir_attributes(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
    };

    std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0 | FILE_WRITE_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_reports_the_same_identity_as_its_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"payload").expect("write");

        let pinned = open_pinned(&path).expect("pin");
        assert_eq!(
            handle_identity(&pinned).expect("identity from the handle"),
            super::super::file_identity(&path).expect("identity from the path"),
        );
        assert!(!is_reparse_point(&pinned).expect("attributes"));
    }

    /// The property the cross-volume move relies on, in both directions: a
    /// writer that is already there keeps the pin out, and a pin keeps any
    /// later writer out.
    #[cfg(windows)]
    #[test]
    fn a_pin_and_a_writer_exclude_each_other() {
        const SHARING_VIOLATION: i32 = 32;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"payload").expect("write");

        let writer = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("writer");
        let refused = open_pinned(&path).expect_err("a writer is already there");
        assert_eq!(refused.raw_os_error(), Some(SHARING_VIOLATION));
        drop(writer);

        let _pinned = open_pinned(&path).expect("pin once the writer is gone");
        let refused = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect_err("the pin keeps writers out");
        assert_eq!(refused.raw_os_error(), Some(SHARING_VIOLATION));
        assert_eq!(
            std::fs::read(&path).expect("readers still get in"),
            b"payload"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_file_opened_for_deleting_is_still_identified() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"payload").expect("write");

        let handle = open_to_delete(&path).expect("open to delete");
        assert_eq!(
            handle_identity(&handle).expect("identity"),
            super::super::file_identity(&path).expect("identity from the path"),
        );
        drop(handle);
        assert!(path.exists(), "opening to delete removes nothing by itself");
    }

    /// The main stream is not listed, a named one is, by its full name and
    /// size; a directory has none.
    #[cfg(windows)]
    #[test]
    fn only_named_streams_are_listed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.exe");
        std::fs::write(&path, b"payload").expect("write");
        let plain = named_streams(&File::open(&path).expect("open")).expect("streams");
        assert!(plain.is_empty(), "the main stream is not listed: {plain:?}");

        let mut zone = path.as_os_str().to_os_string();
        zone.push(":Zone.Identifier");
        std::fs::write(&zone, b"[ZoneTransfer]").expect("named stream");
        let marked = named_streams(&File::open(&path).expect("open")).expect("streams");
        assert_eq!(
            marked,
            vec![DataStream {
                name: ":Zone.Identifier:$DATA".to_owned(),
                size: 14,
            }]
        );

        let folder = named_streams(&open_pinned_dir(dir.path()).expect("dir")).expect("streams");
        assert!(folder.is_empty(), "a plain directory has none: {folder:?}");
    }

    /// The containment check compares final paths; one taken from a handle
    /// must name the same place the walk started from.
    #[cfg(windows)]
    #[test]
    fn a_final_path_names_where_the_handle_points() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"payload").expect("write");

        let root = final_path(&open_pinned_dir(dir.path()).expect("dir"))
            .expect("final path")
            .expect("reported on Windows");
        let file = final_path(&open_pinned(&path).expect("pin"))
            .expect("final path")
            .expect("reported on Windows");
        assert!(root.to_string_lossy().starts_with(r"\\?\"), "{root:?}");
        assert_eq!(file.parent(), Some(root.as_path()));
        assert!(file
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("a.bin")));
    }
}
