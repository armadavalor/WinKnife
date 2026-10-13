//! **Does the File Unlocker name a process that really holds a file?**
//!
//! Unit tests drive the Restart Manager wrapper through a fake. This test asks
//! the real one: a child process (PowerShell, present on every Windows image
//! including GitHub's `windows-latest`) opens a temp file with no sharing and
//! sleeps; the query must return a row with that child's PID.
//!
//! Read-only on our side: the query never closes anything, and the only process
//! ended is the test's own child, in cleanup.
#![cfg(windows)]

use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use winknife_lib::core::platform::real_restart_manager;
use winknife_lib::modules::file_unlocker;

/// How long the child gets to open the file.
const LOCK_WAIT: Duration = Duration::from_secs(30);
/// Poll interval while waiting for the lock.
const POLL: Duration = Duration::from_millis(100);

/// Kills the test's own child on every exit path.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Whether `path` can no longer be opened exclusively (someone holds it).
fn is_locked(path: &Path) -> bool {
    OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .is_err()
}

fn spawn_holder(path: &Path) -> ChildGuard {
    let escaped = path.display().to_string().replace('\'', "''");
    let script = format!(
        "$f = [System.IO.File]::Open('{escaped}', 'Open', 'ReadWrite', 'None'); Start-Sleep -Seconds 120"
    );
    let child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn powershell");
    ChildGuard(child)
}

#[test]
fn query_names_the_child_that_holds_the_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("held.txt");
    std::fs::write(&file, b"held by a child process").expect("write temp file");

    let mut holder = spawn_holder(&file);
    let pid = holder.0.id();

    let deadline = Instant::now() + LOCK_WAIT;
    while !is_locked(&file) {
        if let Some(status) = holder.0.try_wait().expect("poll child") {
            panic!("holder exited early: {status}");
        }
        assert!(Instant::now() < deadline, "child never locked the file");
        std::thread::sleep(POLL);
    }

    let rm = real_restart_manager();
    let report = file_unlocker::query(rm.as_ref(), std::slice::from_ref(&file))
        .expect("restart manager query");

    assert_eq!(report.files_checked, 1);
    let row = report
        .rows
        .iter()
        .find(|row| row.pid == pid)
        .unwrap_or_else(|| panic!("pid {pid} not reported: {:?}", report.rows));
    assert!(row.attributed);
    assert_eq!(row.held_count, 1);
    assert!(!row.reason.is_empty(), "every row carries a reason");
    assert!(row.can_close, "a console app may be asked to close");

    drop(holder);
}
