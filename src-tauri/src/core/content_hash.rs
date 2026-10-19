//! blake3 of a file's contents, for the two places that need to agree about it.
//!
//! # Why this is in `core` and not in the duplicate finder
//!
//! `modules::duplicates` computes it to decide that two files are the same, and
//! `core::executor` computes it again at execute time to confirm the file it is
//! about to quarantine is *still* the one that decision was made about (review
//! H10). Those two numbers have to be produced by the same code, or the check is
//! comparing a hash against a differently-computed hash and will disagree for
//! reasons that have nothing to do with the file.
//!
//! It also keeps the dependency arrow pointing the right way: the executor is
//! not allowed to reach up into a module, and hashing bytes is not a module's
//! private business.

use std::io::Read;
use std::path::Path;

/// How much of the file is read at a time. Large enough that the syscall
/// overhead disappears, small enough that a multi-gigabyte file never lands in
/// memory.
const CHUNK_BYTES: usize = 128 * 1024;

/// blake3 of the whole file, hex-encoded lower-case.
///
/// Read in chunks, so file size does not become memory use.
///
/// # Errors
/// Returns the underlying I/O error if the file cannot be opened or read. A
/// caller that cannot read a file must not conclude anything about its
/// contents — both callers here treat an error as "cannot confirm", never as
/// "confirmed".
pub fn full_hash(path: &Path) -> std::io::Result<String> {
    full_hash_cancellable(path, &|| false)
}

/// [`full_hash`] that asks `is_cancelled` before every chunk.
///
/// # Errors
/// Returns an [`std::io::ErrorKind::Interrupted`] error as soon as
/// `is_cancelled` answers true, so a multi-gigabyte file does not keep a
/// cancelled scan alive until it ends. Otherwise as [`full_hash`].
pub fn full_hash_cancellable(
    path: &Path,
    is_cancelled: &dyn Fn() -> bool,
) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        if is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "hash cancelled",
            ));
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write");
        path
    }

    #[test]
    fn identical_bytes_hash_identically_and_different_bytes_do_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = write(dir.path(), "a.bin", b"the same contents");
        let b = write(dir.path(), "b.bin", b"the same contents");
        let c = write(dir.path(), "c.bin", b"the same contents!");

        assert_eq!(full_hash(&a).expect("hash"), full_hash(&b).expect("hash"));
        assert_ne!(full_hash(&a).expect("hash"), full_hash(&c).expect("hash"));
    }

    /// The chunk boundary is the one place a streaming hash can go wrong, and a
    /// difference past it is exactly what the executor's re-check has to catch.
    #[test]
    fn a_file_larger_than_one_chunk_hashes_over_all_of_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let size = CHUNK_BYTES * 3 + 17;

        let a = write(dir.path(), "big-a.bin", &vec![0xAB; size]);
        let mut tail_differs = vec![0xAB; size];
        tail_differs[size - 1] = 0xCD;
        let b = write(dir.path(), "big-b.bin", &tail_differs);
        let mut middle_differs = vec![0xAB; size];
        middle_differs[size / 2] = 0xCD;
        let c = write(dir.path(), "big-c.bin", &middle_differs);

        let hash_a = full_hash(&a).expect("hash");
        assert_ne!(hash_a, full_hash(&b).expect("hash"), "a changed last byte");
        assert_ne!(
            hash_a,
            full_hash(&c).expect("hash"),
            "a changed middle byte"
        );
    }

    #[test]
    fn an_empty_file_hashes_rather_than_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let empty = write(dir.path(), "empty.bin", b"");
        assert!(!full_hash(&empty).expect("hash").is_empty());
    }

    #[test]
    fn a_missing_file_is_an_error_and_never_a_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(full_hash(&dir.path().join("nope.bin")).is_err());
    }

    #[test]
    fn a_hash_cancelled_from_the_start_is_interrupted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "a.bin", b"contents");
        let err = full_hash_cancellable(&file, &|| true).expect_err("cancelled");
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn a_hash_cancelled_between_chunks_is_interrupted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "big.bin", &vec![1u8; CHUNK_BYTES * 3]);
        let calls = std::cell::Cell::new(0);
        let err = full_hash_cancellable(&file, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 1
        })
        .expect_err("cancelled");
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn the_cancellable_path_hashes_the_same_as_the_plain_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "a.bin", &vec![7u8; CHUNK_BYTES + 5]);
        assert_eq!(
            full_hash_cancellable(&file, &|| false).expect("hash"),
            full_hash(&file).expect("hash")
        );
    }
}
