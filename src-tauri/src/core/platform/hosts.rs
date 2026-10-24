//! The one write WinKnife makes under `C:\Windows`: the hosts file (K-019).
//!
//! The app (unelevated) never writes the hosts file itself. It stages the new
//! content in app data and re-launches its own exe elevated with
//! [`WRITE_HOSTS_FLAG`], like the restore-point helper. The elevated copy
//! resolves the target from `%SystemRoot%` on its own — the target is never a
//! command-line argument — checks the staged bytes against the hash the app
//! sent, checks the file still holds what the app last read, and writes through
//! [`write_atomic`]: a temp file in the same folder, an atomic swap, then a
//! read-back. Every write passes [`HostsFence::check`], a canonical
//! path-equality check against that one resolved path, so nothing else under
//! `C:\Windows` can be reached through this code.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::restore_point::{failed_exit_code, run_elevated, HelperRun};
use super::EnvResolver;
use crate::core::error::{CoreError, Result};
use crate::core::path_compare::same_path;

/// Command-line flag that turns the exe into the elevated hosts writer.
pub const WRITE_HOSTS_FLAG: &str = "--write-hosts";
/// The helper wrote the file and read it back byte for byte.
pub const EXIT_WRITTEN: u32 = 0;
/// The staged content was empty, unreadable or did not match its hash.
pub const EXIT_INVALID: u32 = 2;
/// The hosts file changed since the app read it; nothing was written.
pub const EXIT_CHANGED: u32 = 3;
/// The swap happened but the read-back differed from what was staged.
pub const EXIT_VERIFY_FAILED: u32 = 4;
/// The exact command the "Flush DNS cache" button runs, shown in the UI.
pub const FLUSH_DNS_COMMAND: &str = "ipconfig /flushdns";

/// Stands for "there was no hosts file when the app read it".
const NO_FILE: &str = "-";
/// `ERROR_ACCESS_DENIED`, for a helper started from a build that may not act.
const WIN32_ACCESS_DENIED: u32 = 5;
/// `ERROR_INVALID_DATA`, for an I/O error that carries no Win32 code.
const WIN32_INVALID_DATA: u32 = 13;
/// `CREATE_NO_WINDOW`: `ipconfig` must not flash a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The hash the app and the helper agree on for a hosts file's bytes.
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// The only path the hosts writer may touch.
#[derive(Debug, Clone)]
pub struct HostsFence {
    path: PathBuf,
}

impl HostsFence {
    /// `%SystemRoot%\System32\drivers\etc\hosts`.
    ///
    /// # Errors
    /// Returns an error when `%SystemRoot%` is unset or not an absolute path.
    pub fn system(env: &dyn EnvResolver) -> Result<Self> {
        let root = env
            .var("SystemRoot")
            .filter(|root| !root.trim().is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| CoreError::Hosts("%SystemRoot% is not set".to_owned()))?;
        if !root.is_absolute() {
            return Err(CoreError::Hosts(format!(
                "%SystemRoot% is not an absolute path: {}",
                root.display()
            )));
        }
        Ok(Self {
            path: root
                .join("System32")
                .join("drivers")
                .join("etc")
                .join("hosts"),
        })
    }

    /// A fence around `path`, for tests that write to a tempdir.
    #[cfg(test)]
    pub(crate) fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The fenced hosts file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Refuse every target that is not the fenced file.
    ///
    /// # Errors
    /// Returns [`CoreError::Hosts`] for any other path.
    pub fn check(&self, target: &Path) -> Result<()> {
        if same_path(target, &self.path) {
            Ok(())
        } else {
            Err(CoreError::Hosts(format!(
                "refusing to write {}: only {} may be written",
                target.display(),
                self.path.display()
            )))
        }
    }
}

/// Replace the fenced file with `bytes` atomically, then read it back.
///
/// # Errors
/// Returns [`CoreError::Hosts`] for a path outside the fence or empty content,
/// [`CoreError::VerifyFailed`] when the read-back differs, or the I/O error.
pub fn write_atomic(fence: &HostsFence, target: &Path, bytes: &[u8]) -> Result<()> {
    fence.check(target)?;
    if bytes.is_empty() {
        return Err(CoreError::Hosts(
            "refusing to write an empty hosts file".to_owned(),
        ));
    }
    let dir = target
        .parent()
        .ok_or_else(|| CoreError::Hosts("the hosts file has no folder".to_owned()))?;
    // One fixed name: the swap consumes it, and a temp a failed swap leaves
    // behind is overwritten by the next save instead of being deleted (only
    // quarantine removes files).
    let temp = dir.join("hosts.winknife.tmp");
    write_temp(&temp, bytes).and_then(|()| sys::swap(&temp, target))?;
    if fs::read(target)? != bytes {
        return Err(CoreError::VerifyFailed(format!(
            "{} does not hold what was written",
            target.display()
        )));
    }
    Ok(())
}

// Truncating is fine here: `temp` is only ever our own `hosts.winknife.tmp`.
#[allow(clippy::disallowed_methods)]
fn write_temp(temp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(temp)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The elevated helper's whole job, against `fence`: write `staged` over the
/// hosts file if it is what the app validated and the file is unchanged.
pub fn helper_write(
    fence: &HostsFence,
    staged: &Path,
    expect_current: Option<&str>,
    staged_hash: &str,
) -> u32 {
    let bytes = match fs::read(staged) {
        Ok(bytes) => bytes,
        Err(err) => return io_exit_code(&err),
    };
    if bytes.is_empty() || content_hash(&bytes) != staged_hash {
        return EXIT_INVALID;
    }
    let current = match fs::read(fence.path()) {
        Ok(current) => Some(content_hash(&current)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return io_exit_code(&err),
    };
    if current.as_deref() != expect_current {
        return EXIT_CHANGED;
    }
    match write_atomic(fence, fence.path(), &bytes) {
        Ok(()) => EXIT_WRITTEN,
        Err(CoreError::VerifyFailed(_)) => EXIT_VERIFY_FAILED,
        Err(CoreError::Io(err)) => io_exit_code(&err),
        Err(_) => EXIT_INVALID,
    }
}

fn io_exit_code(err: &std::io::Error) -> u32 {
    let code = err
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
        .unwrap_or(WIN32_INVALID_DATA);
    failed_exit_code(code)
}

/// When this process was started as the hosts writer, do that job and return
/// its exit code; `None` for a normal launch.
pub fn run_helper_if_asked(args: impl IntoIterator<Item = String>) -> Option<u32> {
    let mut args = args.into_iter().skip(1);
    if args.next().as_deref() != Some(WRITE_HOSTS_FLAG) {
        return None;
    }
    let (Some(staged), Some(expect), Some(hash)) = (args.next(), args.next(), args.next()) else {
        return Some(EXIT_INVALID);
    };
    if !super::real_os_effects_armed() {
        return Some(failed_exit_code(WIN32_ACCESS_DENIED));
    }
    let Ok(fence) = HostsFence::system(&super::SystemEnv) else {
        return Some(EXIT_INVALID);
    };
    let expect = (expect != NO_FILE).then_some(expect.as_str());
    Some(helper_write(&fence, Path::new(&staged), expect, &hash))
}

/// Writes staged hosts content; the real one goes through the UAC helper.
pub trait HostsWriter: Send + Sync {
    /// Ask the helper to write `staged`, which hashes to `staged_hash`, if the
    /// hosts file still hashes to `expect_current` (`None`: no file).
    ///
    /// # Errors
    /// Returns an error when this build is not allowed to reach the OS.
    fn write(
        &self,
        staged: &Path,
        expect_current: Option<&str>,
        staged_hash: &str,
    ) -> Result<HelperRun>;
}

/// The real writer: re-launches WinKnife elevated with [`WRITE_HOSTS_FLAG`].
#[derive(Debug, Default)]
pub struct ElevatedHostsWriter;

impl HostsWriter for ElevatedHostsWriter {
    fn write(
        &self,
        staged: &Path,
        expect_current: Option<&str>,
        staged_hash: &str,
    ) -> Result<HelperRun> {
        if !super::real_os_effects_armed() {
            return Err(CoreError::Hosts(
                "this build must never start an elevated helper".to_owned(),
            ));
        }
        // A Windows path cannot contain `"`, and the hashes are hex.
        run_elevated(&format!(
            "{WRITE_HOSTS_FLAG} \"{}\" {} {staged_hash}",
            staged.display(),
            expect_current.unwrap_or(NO_FILE)
        ))
    }
}

/// Run [`FLUSH_DNS_COMMAND`] and return what it printed. Only on a click.
///
/// # Errors
/// Returns an error when this build may not reach the OS, `ipconfig` could not
/// be started, or it reported a failure.
pub fn flush_dns(env: &dyn EnvResolver) -> Result<String> {
    if !super::real_os_effects_armed() {
        return Err(CoreError::Hosts(
            "this build must never run ipconfig".to_owned(),
        ));
    }
    let root = env
        .var("SystemRoot")
        .ok_or_else(|| CoreError::Hosts("%SystemRoot% is not set".to_owned()))?;
    // The full path, so a planted `ipconfig.exe` on PATH is never run.
    let exe = Path::new(&root).join("System32").join("ipconfig.exe");
    let mut command = std::process::Command::new(exe);
    command.arg("/flushdns");
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, CREATE_NO_WINDOW);
    let output = command.output()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() {
        Ok(text)
    } else {
        Err(CoreError::Hosts(format!(
            "{FLUSH_DNS_COMMAND} failed: {text}"
        )))
    }
}

#[cfg(windows)]
mod sys {
    use std::path::Path;

    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{
        MoveFileW, ReplaceFileW, REPLACEFILE_IGNORE_MERGE_ERRORS,
    };

    /// Put `temp` in place of `target`. `ReplaceFileW` keeps the hosts file's
    /// own permissions and attributes; a missing target falls back to a plain
    /// move, which never replaces anything (only quarantine may).
    pub(super) fn swap(temp: &Path, target: &Path) -> std::io::Result<()> {
        let temp = HSTRING::from(temp.as_os_str());
        let target_name = HSTRING::from(target.as_os_str());
        // SAFETY: both strings outlive the call; no backup file, no reserved args.
        let result = if target.exists() {
            unsafe {
                ReplaceFileW(
                    &target_name,
                    &temp,
                    None,
                    REPLACEFILE_IGNORE_MERGE_ERRORS,
                    None,
                    None,
                )
            }
        } else {
            unsafe { MoveFileW(&temp, &target_name) }
        };
        result.map_err(|err| std::io::Error::from_raw_os_error(err.code().0 & 0xFFFF))
    }
}

#[cfg(not(windows))]
mod sys {
    use std::path::Path;

    pub(super) fn swap(temp: &Path, target: &Path) -> std::io::Result<()> {
        std::fs::rename(temp, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::MapEnv;

    fn fenced(dir: &tempfile::TempDir) -> HostsFence {
        HostsFence::at(dir.path().join("hosts"))
    }

    fn stage(dir: &tempfile::TempDir, bytes: &[u8]) -> (PathBuf, String) {
        let staged = dir.path().join("staged");
        fs::write(&staged, bytes).expect("stage");
        (staged, content_hash(bytes))
    }

    #[test]
    fn system_fence_is_resolved_from_system_root() {
        let env = MapEnv::new().with("SystemRoot", "C:\\Windows");
        let fence = HostsFence::system(&env).expect("fence");
        assert!(same_path(
            fence.path(),
            Path::new("C:\\Windows\\System32\\drivers\\etc\\hosts")
        ));
    }

    #[test]
    fn system_fence_needs_an_absolute_system_root() {
        assert!(HostsFence::system(&MapEnv::new()).is_err());
        let relative = MapEnv::new().with("SystemRoot", "Windows");
        assert!(HostsFence::system(&relative).is_err());
    }

    #[test]
    fn fence_refuses_every_other_path() {
        let env = MapEnv::new().with("SystemRoot", "C:\\Windows");
        let fence = HostsFence::system(&env).expect("fence");
        for other in [
            "C:\\Windows\\System32\\drivers\\etc\\hosts.bak",
            "C:\\Windows\\System32\\drivers\\etc\\lmhosts.sam",
            "C:\\Windows\\System32\\drivers\\etc",
            "C:\\Windows\\System32\\config\\SAM",
            "C:\\Windows\\System32\\drivers\\etc\\hosts\\..\\networks",
            "D:\\Windows\\System32\\drivers\\etc\\hosts",
        ] {
            assert!(fence.check(Path::new(other)).is_err(), "{other}");
        }
        assert!(fence
            .check(Path::new("c:\\windows\\system32\\DRIVERS\\etc\\HOSTS"))
            .is_ok());
    }

    #[test]
    fn write_atomic_refuses_a_path_outside_the_fence_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fence = fenced(&dir);
        let other = dir.path().join("networks");
        assert!(write_atomic(&fence, &other, b"x\n").is_err());
        assert!(!other.exists());
    }

    #[test]
    fn write_atomic_refuses_empty_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fence = fenced(&dir);
        fs::write(fence.path(), b"127.0.0.1 a\n").expect("seed");
        assert!(write_atomic(&fence, fence.path(), b"").is_err());
        assert_eq!(fs::read(fence.path()).expect("read"), b"127.0.0.1 a\n");
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fence = fenced(&dir);
        fs::write(fence.path(), b"old\r\n").expect("seed");
        write_atomic(&fence, fence.path(), b"127.0.0.1 new\r\n").expect("write");
        assert_eq!(fs::read(fence.path()).expect("read"), b"127.0.0.1 new\r\n");
        let names: Vec<_> = fs::read_dir(dir.path())
            .expect("dir")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["hosts"]);
    }

    #[test]
    fn write_atomic_creates_a_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fence = fenced(&dir);
        write_atomic(&fence, fence.path(), b"127.0.0.1 a\n").expect("write");
        assert_eq!(fs::read(fence.path()).expect("read"), b"127.0.0.1 a\n");
    }

    #[test]
    fn helper_writes_when_the_file_is_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fence = fenced(&dir);
        fs::write(fence.path(), b"old\n").expect("seed");
        let (staged, hash) = stage(&dir, b"127.0.0.1 new\n");
        let expect = content_hash(b"old\n");
        assert_eq!(
            helper_write(&fence, &staged, Some(&expect), &hash),
            EXIT_WRITTEN
        );
        assert_eq!(fs::read(fence.path()).expect("read"), b"127.0.0.1 new\n");
    }

    #[test]
    fn helper_refuses_a_file_that_changed_since_it_was_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fence = fenced(&dir);
        fs::write(fence.path(), b"changed\n").expect("seed");
        let (staged, hash) = stage(&dir, b"127.0.0.1 new\n");
        let expect = content_hash(b"old\n");
        assert_eq!(
            helper_write(&fence, &staged, Some(&expect), &hash),
            EXIT_CHANGED
        );
        assert_eq!(helper_write(&fence, &staged, None, &hash), EXIT_CHANGED);
        assert_eq!(fs::read(fence.path()).expect("read"), b"changed\n");
    }

    #[test]
    fn helper_refuses_staged_bytes_that_do_not_match_their_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fence = fenced(&dir);
        fs::write(fence.path(), b"old\n").expect("seed");
        let (staged, _) = stage(&dir, b"127.0.0.1 new\n");
        let expect = content_hash(b"old\n");
        let wrong = content_hash(b"something else");
        assert_eq!(
            helper_write(&fence, &staged, Some(&expect), &wrong),
            EXIT_INVALID
        );
        let (empty, empty_hash) = stage(&dir, b"");
        assert_eq!(
            helper_write(&fence, &empty, Some(&expect), &empty_hash),
            EXIT_INVALID
        );
        assert_eq!(fs::read(fence.path()).expect("read"), b"old\n");
    }

    #[test]
    fn a_normal_launch_is_not_the_helper() {
        let args = ["winknife.exe".to_owned(), "--module".to_owned()];
        assert_eq!(run_helper_if_asked(args), None);
    }

    #[test]
    fn the_helper_refuses_in_a_build_that_may_not_act() {
        let args = [
            "winknife.exe",
            WRITE_HOSTS_FLAG,
            "C:\\staged",
            NO_FILE,
            "abc",
        ]
        .map(str::to_owned);
        assert_eq!(
            run_helper_if_asked(args),
            Some(failed_exit_code(WIN32_ACCESS_DENIED))
        );
        let short = ["winknife.exe", WRITE_HOSTS_FLAG].map(str::to_owned);
        assert_eq!(run_helper_if_asked(short), Some(EXIT_INVALID));
    }

    #[test]
    fn the_real_writer_and_flush_refuse_in_tests() {
        let env = MapEnv::new().with("SystemRoot", "C:\\Windows");
        assert!(ElevatedHostsWriter
            .write(Path::new("C:\\staged"), None, "abc")
            .is_err());
        assert!(flush_dns(&env).is_err());
    }
}
