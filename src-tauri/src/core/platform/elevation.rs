//! Whether WinKnife runs elevated, and restarting it as administrator on request.
//!
//! WinKnife starts with the user's own rights (per-user install, no admin
//! manifest). A few actions need more: HKLM startup entries, the all-users
//! Startup folder, tasks owned by Windows or by other accounts. Those fail with
//! "access denied", and the page offers a restart through the UAC prompt —
//! only on a click, never at startup (K-020).
//!
//! # The single-instance plugin
//!
//! A second WinKnife forwards its argv to the running one and exits. The
//! elevated copy would be turned away the same way, so it is started with
//! [`AFTER_ELEVATION_FLAG`] and the old process id, and waits — before the
//! plugin runs — for that process to finish exiting.

use std::time::Duration;

use serde::Serialize;

use crate::core::error::{CoreError, Result};

/// `--after-elevation <pid>`: wait for the instance that asked for this restart.
pub const AFTER_ELEVATION_FLAG: &str = "--after-elevation";

/// How long the elevated copy waits for the old one to close. After that it
/// starts anyway; at worst the single-instance plugin hands its argv over and
/// the old window comes back.
pub const PREVIOUS_INSTANCE_TIMEOUT: Duration = Duration::from_secs(10);

/// How a "Restart as administrator" request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum RelaunchOutcome {
    /// The elevated copy is starting; this one is about to exit.
    Started,
    /// The user answered No at the UAC prompt. Nothing changed.
    Declined,
    /// Windows could not start it; the Win32 error.
    Failed { code: u32 },
}

/// The command line for the elevated copy: the page it should open, the
/// dry-run flag if this run has it, and the pid to wait for.
///
/// A module id is kept only when it is a plain id (`[A-Za-z0-9_-]`): it ends up
/// on a command line, so anything that could break out of its argument is
/// dropped and the copy opens the dashboard instead.
pub fn relaunch_parameters(module: Option<&str>, dry_run: bool, pid: u32) -> String {
    let mut parts = Vec::new();
    if let Some(id) = module.filter(|id| is_plain_id(id)) {
        parts.push(format!("{} {id}", crate::MODULE_FLAG));
    }
    if dry_run {
        parts.push(crate::DRY_RUN_FLAG.to_owned());
    }
    parts.push(format!("{AFTER_ELEVATION_FLAG} {pid}"));
    parts.join(" ")
}

fn is_plain_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The pid after [`AFTER_ELEVATION_FLAG`], if this launch has one.
pub fn previous_instance(args: impl IntoIterator<Item = String>) -> Option<u32> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == AFTER_ELEVATION_FLAG {
            return args.next().and_then(|pid| pid.parse().ok());
        }
    }
    None
}

/// Whether this process runs with an elevated (administrator) token. Read-only;
/// `false` when Windows will not say.
pub fn is_elevated() -> bool {
    sys::is_elevated()
}

/// Start this exe again through `runas` with `parameters`. Does not wait.
///
/// # Errors
/// Returns an error when this build may not reach the OS, or when WinKnife's
/// own exe cannot be located.
pub fn relaunch_elevated(parameters: &str) -> Result<RelaunchOutcome> {
    if !super::real_os_effects_armed() {
        return Err(CoreError::ElevationBlocked);
    }
    sys::relaunch_elevated(parameters)
}

/// Wait up to [`PREVIOUS_INSTANCE_TIMEOUT`] for process `pid` to exit. A pid
/// that is already gone, or cannot be opened, returns at once.
pub fn wait_for_exit(pid: u32) {
    sys::wait_for_exit(pid, PREVIOUS_INSTANCE_TIMEOUT);
}

#[cfg(windows)]
mod sys {
    use super::RelaunchOutcome;
    use crate::core::error::Result;
    use crate::core::platform::WIN32_CANCELLED;
    use std::time::Duration;
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    pub(super) fn is_elevated() -> bool {
        let mut token = HANDLE::default();
        // SAFETY: the pseudo-handle needs no closing; `token` is a valid out-pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0_u32;
        let size = u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(u32::MAX);
        // SAFETY: a live token handle and a buffer of exactly `size` bytes.
        let read = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                Some(std::ptr::from_mut(&mut elevation).cast()),
                size,
                &mut returned,
            )
        };
        // SAFETY: opened above.
        let _ = unsafe { CloseHandle(token) };
        read.is_ok() && elevation.TokenIsElevated != 0
    }

    pub(super) fn relaunch_elevated(parameters: &str) -> Result<RelaunchOutcome> {
        let exe = std::env::current_exe()?;
        let file = HSTRING::from(exe.as_os_str());
        let parameters = HSTRING::from(parameters);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or(u32::MAX),
            fMask: SEE_MASK_NOASYNC,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: every pointer in `info` outlives the call.
        match unsafe { ShellExecuteExW(&mut info) } {
            Ok(()) => Ok(RelaunchOutcome::Started),
            Err(err) => {
                let hr = err.code().0 as u32;
                let code = if hr & 0xFFFF_0000 == 0x8007_0000 {
                    hr & 0xFFFF
                } else {
                    hr
                };
                Ok(if code == WIN32_CANCELLED {
                    RelaunchOutcome::Declined
                } else {
                    RelaunchOutcome::Failed { code }
                })
            }
        }
    }

    pub(super) fn wait_for_exit(pid: u32, timeout: Duration) {
        // SAFETY: SYNCHRONIZE only; a pid that is gone fails to open.
        let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
            return;
        };
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: a live process handle, closed right after.
        unsafe {
            let _ = WaitForSingleObject(process, millis);
            let _ = CloseHandle(process);
        }
    }
}

#[cfg(not(windows))]
mod sys {
    use super::RelaunchOutcome;
    use crate::core::error::Result;
    use std::time::Duration;

    /// `ERROR_NOT_SUPPORTED`.
    const NOT_SUPPORTED: u32 = 50;

    pub(super) fn is_elevated() -> bool {
        false
    }

    pub(super) fn relaunch_elevated(_parameters: &str) -> Result<RelaunchOutcome> {
        Ok(RelaunchOutcome::Failed {
            code: NOT_SUPPORTED,
        })
    }

    pub(super) fn wait_for_exit(_pid: u32, _timeout: Duration) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parameters_keep_the_page_and_dry_run_and_name_the_old_pid() {
        assert_eq!(
            relaunch_parameters(Some("startup"), true, 4242),
            "--module startup --dry-run --after-elevation 4242"
        );
        assert_eq!(relaunch_parameters(None, false, 7), "--after-elevation 7");
    }

    #[test]
    fn a_module_id_that_could_break_out_of_its_argument_is_dropped() {
        for bad in ["", "a b", "x\" --evil", "..\\x", "start&calc"] {
            assert_eq!(
                relaunch_parameters(Some(bad), false, 1),
                "--after-elevation 1",
                "{bad:?}"
            );
        }
        assert_eq!(
            relaunch_parameters(Some("empty-folders"), false, 1),
            "--module empty-folders --after-elevation 1"
        );
    }

    #[test]
    fn the_relaunched_copy_reads_back_what_it_was_given() {
        let line = relaunch_parameters(Some("hosts"), true, 99);
        let argv: Vec<String> = std::iter::once("winknife.exe".to_owned())
            .chain(line.split(' ').map(str::to_owned))
            .collect();
        assert_eq!(previous_instance(argv.clone()), Some(99));
        assert_eq!(
            crate::parse_launch_args(argv.clone()).module.as_deref(),
            Some("hosts")
        );
        assert!(crate::is_dry_run(argv));
    }

    #[test]
    fn a_missing_or_garbled_pid_means_no_wait() {
        assert_eq!(previous_instance(args(&["app.exe"])), None);
        assert_eq!(
            previous_instance(args(&["app.exe", AFTER_ELEVATION_FLAG])),
            None
        );
        assert_eq!(
            previous_instance(args(&["app.exe", AFTER_ELEVATION_FLAG, "abc"])),
            None
        );
    }

    #[test]
    fn a_test_build_never_starts_an_elevated_copy() {
        let err = relaunch_elevated("--after-elevation 1").expect_err("guarded");
        assert_eq!(err.code(), "elevation_blocked");
    }

    #[test]
    fn asking_whether_we_are_elevated_does_not_panic() {
        let _ = is_elevated();
    }
}
