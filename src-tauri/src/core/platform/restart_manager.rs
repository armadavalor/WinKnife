//! The Windows Restart Manager, behind a trait (module spec 07).
//!
//! Two questions go through here: "which programs have these files open?" and
//! "please ask this one program to close". The first is read-only. The second is
//! the only call in the app that reaches another program, so its shape is
//! deliberately narrow:
//!
//! - [`RmApi::shutdown`] has no flags parameter. The system implementation
//!   passes `0` to `RmShutdown` — never `RmForceShutdown`, never
//!   `RmShutdownOnlyRegistered` — and nothing here calls `TerminateProcess` or
//!   closes another process's handles. A program that declines simply stays
//!   open.
//! - A close uses a fresh session that registers only that one process by
//!   PID **and** start time, so a recycled PID can never be the one asked.
//! - The process is re-classified inside that fresh session; services,
//!   critical processes, Explorer, the System process and WinKnife itself are
//!   refused here, not only in the UI.
//! - The real `RmShutdown` refuses until
//!   [`real_os_effects_armed`](super::real_os_effects_armed) — no test binary
//!   can ask a real program to close.
//!
//! The session handling lives in [`RmSessions`], generic over the raw
//! [`RmApi`], so the guard ("`RmEndSession` always runs"), the 500-file batches
//! and the bounded `ERROR_MORE_DATA` retry are unit-tested against a fake.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

mod timed;
pub use timed::{TimedRestartManager, CLOSE_TIMEOUT, QUERY_TIMEOUT};

/// Files handed to one `RmRegisterResources` call.
pub const REGISTER_BATCH: usize = 500;

/// How many times `RmGetList` is asked again after `ERROR_MORE_DATA`.
///
/// The list grows when a program opens one of the files between the size
/// query and the fetch. Three more tries is plenty for a human-scale race and
/// still bounds a pathological one.
pub const MORE_DATA_RETRIES: usize = 3;

/// Room for this many entries on the first `RmGetList` call.
const INITIAL_LIST_CAPACITY: usize = 32;

/// Extra room added on a retry, so one more opener does not cost another round.
const LIST_SLACK: usize = 8;

/// The PID Windows gives the kernel's "System" process.
pub const SYSTEM_PID: u32 = 4;

/// A process named the way the Restart Manager names it: PID plus start time.
///
/// The start time is the process creation `FILETIME` as one `u64`
/// (`high << 32 | low`). Two processes can share a PID over time; they cannot
/// share both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UniqueProcess {
    /// Process id.
    pub pid: u32,
    /// Creation time as a 64-bit `FILETIME`.
    pub start_time: u64,
}

/// What kind of program the Restart Manager says this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    /// A program with a top-level window (`RmMainWindow`, `RmOtherWindow`).
    WindowApp,
    /// A console program (`RmConsole`).
    Console,
    /// A Windows service (`RmService`).
    Service,
    /// Windows Explorer (`RmExplorer`).
    Explorer,
    /// A process Windows needs to keep running (`RmCritical`).
    Critical,
    /// The Restart Manager could not tell (`RmUnknownApp`, or anything newer).
    Unknown,
}

impl AppKind {
    /// Map the raw `RM_APP_TYPE` value.
    pub fn from_rm(raw: i32) -> Self {
        match raw {
            1 | 2 => Self::WindowApp,
            3 => Self::Service,
            4 => Self::Explorer,
            5 => Self::Console,
            1000 => Self::Critical,
            _ => Self::Unknown,
        }
    }
}

/// One program the Restart Manager says holds a registered resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockInfo {
    /// Which process.
    pub process: UniqueProcess,
    /// The friendly name Windows reports (`strAppName`).
    pub app_name: String,
    /// The service short name, for a service.
    pub service_name: Option<String>,
    /// What kind of program it is.
    pub kind: AppKind,
}

/// Whether a holder may be asked to close, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosePolicy {
    /// A window or console program; it may be asked politely.
    Closable {
        /// Stable code for the UI's translation.
        code: &'static str,
        /// Human-readable reason.
        reason: &'static str,
    },
    /// Shown for information only; no close button, and the backend refuses.
    InfoOnly {
        /// Stable code for the UI's translation.
        code: &'static str,
        /// Human-readable reason.
        reason: &'static str,
    },
}

impl ClosePolicy {
    /// Whether the policy allows asking.
    pub fn is_closable(&self) -> bool {
        matches!(self, Self::Closable { .. })
    }

    /// The stable code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Closable { code, .. } | Self::InfoOnly { code, .. } => code,
        }
    }

    /// The human-readable reason.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Closable { reason, .. } | Self::InfoOnly { reason, .. } => reason,
        }
    }
}

/// Decide what may be done about a holder. The single source of truth for both
/// the result rows and the server-side refusal.
pub fn close_policy(pid: u32, kind: AppKind) -> ClosePolicy {
    if pid == SYSTEM_PID {
        return ClosePolicy::InfoOnly {
            code: "system",
            reason: "Windows itself (the System process) holds this, often because the file is shared over the network or being indexed. It cannot be closed.",
        };
    }
    if pid == std::process::id() {
        return ClosePolicy::InfoOnly {
            code: "self",
            reason: "This is WinKnife itself. Close the screen that is using the file instead.",
        };
    }
    match kind {
        AppKind::WindowApp => ClosePolicy::Closable {
            code: "window",
            reason: "Window app. It can be asked to close, the same way Windows asks when you sign out.",
        },
        AppKind::Console => ClosePolicy::Closable {
            code: "console",
            reason: "Console program. It can be asked to close; it may stop whatever it is running.",
        },
        AppKind::Service => ClosePolicy::InfoOnly {
            code: "service",
            reason: "Windows service — stop it from Services if you are sure.",
        },
        AppKind::Explorer => ClosePolicy::InfoOnly {
            code: "explorer",
            reason: "Explorer — restarting it is safer than closing it; WinKnife does not do this for you.",
        },
        AppKind::Critical => ClosePolicy::InfoOnly {
            code: "critical",
            reason: "Critical system process — closing it could crash or restart Windows, so WinKnife never asks.",
        },
        AppKind::Unknown => ClosePolicy::InfoOnly {
            code: "unknown",
            reason: "Windows could not say what kind of program this is, so WinKnife does not offer to close it.",
        },
    }
}

/// What a polite close request came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CloseOutcome {
    /// The process is gone after the request.
    Closed,
    /// It is still running — it declined, is waiting on the user (an unsaved
    /// document), or did not answer in time.
    StillRunning,
    /// It had already exited before anything was asked.
    AlreadyExited,
    /// Nothing was asked, and why.
    Refused {
        /// Human-readable reason.
        reason: String,
    },
}

/// Whether a PID is running, and since when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Running, with this start time.
    Running(u64),
    /// Not running.
    Gone,
    /// Could not be checked (access denied, protected process).
    Unknown,
}

/// What went wrong talking to the Restart Manager.
#[derive(Debug, thiserror::Error)]
pub enum RmError {
    /// A Restart Manager call returned an error code.
    #[error("Windows could not say which programs use these files ({call} returned error {code})")]
    Win32 {
        /// The API that failed.
        call: &'static str,
        /// The `WIN32_ERROR` value.
        code: u32,
    },
    /// The list kept growing past the bounded retry.
    #[error("the list of programs kept changing while it was read; try again")]
    ListKeptGrowing,
    /// This build may not ask real programs to close (every test build).
    #[error("this build is not allowed to ask other programs to close")]
    NotArmed,
    /// No Restart Manager on this platform.
    #[error("the Restart Manager is only available on Windows")]
    Unsupported,
    /// Windows did not answer within the bounded wait (issue #41).
    #[error("Windows did not answer within {seconds} seconds ({call}); it may still finish later, try again")]
    TimedOut {
        /// The API that was waited on.
        call: &'static str,
        /// How long the caller waited.
        seconds: u64,
    },
}

/// One `RmGetList` round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListAttempt {
    /// The whole list fitted.
    Complete(Vec<LockInfo>),
    /// `ERROR_MORE_DATA`: this many entries are needed.
    MoreData {
        /// Entries Windows says it needs room for.
        needed: usize,
    },
}

/// The raw Restart Manager calls, one method per API.
///
/// Implemented by the system wrapper and by a test fake. There is no way to
/// pass shutdown flags: the only shutdown this trait knows is the polite one.
pub trait RmApi: Send + Sync {
    /// `RmStartSession`.
    fn start_session(&self) -> Result<u32, RmError>;
    /// `RmEndSession`. Called exactly once per started session, by [`Session`].
    fn end_session(&self, session: u32);
    /// `RmRegisterResources` with files only. At most [`REGISTER_BATCH`] at a time.
    fn register_files(&self, session: u32, files: &[PathBuf]) -> Result<(), RmError>;
    /// `RmRegisterResources` with exactly one process.
    fn register_process(&self, session: u32, process: UniqueProcess) -> Result<(), RmError>;
    /// One `RmGetList` call with room for `capacity` entries.
    fn get_list(&self, session: u32, capacity: usize) -> Result<ListAttempt, RmError>;
    /// `RmShutdown(session, 0, None)` — polite, no force flag, ever.
    fn shutdown(&self, session: u32) -> Result<(), RmError>;
    /// Whether `pid` runs, and its start time.
    fn process_state(&self, pid: u32) -> ProcessState;
    /// The full executable path of `pid`, when it can be read.
    fn exe_path(&self, pid: u32) -> Option<PathBuf>;
}

/// The two questions the file unlocker asks (module spec 07).
pub trait RestartManager: Send + Sync {
    /// Which programs have any of `paths` open. Read-only.
    ///
    /// # Errors
    /// Returns an error when a Restart Manager call fails.
    fn locking_processes(&self, paths: &[PathBuf]) -> Result<Vec<LockInfo>, RmError>;

    /// Ask one program to close, politely. Refuses on a PID/start-time
    /// mismatch and for every kind [`close_policy`] marks info-only.
    ///
    /// # Errors
    /// Returns an error when a Restart Manager call fails, or when this build
    /// is not allowed to reach the OS.
    fn request_close(&self, process: UniqueProcess) -> Result<CloseOutcome, RmError>;

    /// The executable path of `pid`, when it can be read.
    fn exe_path(&self, pid: u32) -> Option<PathBuf>;
}

/// A started session that ends itself, whatever happens in between.
struct Session<'a, A: RmApi + ?Sized> {
    api: &'a A,
    handle: u32,
}

impl<'a, A: RmApi + ?Sized> Session<'a, A> {
    fn start(api: &'a A) -> Result<Self, RmError> {
        let handle = api.start_session()?;
        Ok(Self { api, handle })
    }
}

impl<A: RmApi + ?Sized> Drop for Session<'_, A> {
    fn drop(&mut self) {
        self.api.end_session(self.handle);
    }
}

/// [`RestartManager`] built on any [`RmApi`].
#[derive(Debug, Default)]
pub struct RmSessions<A: RmApi> {
    api: A,
}

impl<A: RmApi> RmSessions<A> {
    /// Wrap a raw API.
    pub fn new(api: A) -> Self {
        Self { api }
    }

    /// The wrapped API (tests inspect the fake through this).
    #[cfg(test)]
    pub fn api(&self) -> &A {
        &self.api
    }

    fn list_all(&self, session: u32) -> Result<Vec<LockInfo>, RmError> {
        let mut capacity = INITIAL_LIST_CAPACITY;
        for _ in 0..=MORE_DATA_RETRIES {
            match self.api.get_list(session, capacity)? {
                ListAttempt::Complete(list) => return Ok(list),
                ListAttempt::MoreData { needed } => {
                    capacity = needed.max(capacity) + LIST_SLACK;
                }
            }
        }
        Err(RmError::ListKeptGrowing)
    }

    fn refusal(reason: impl Into<String>) -> Result<CloseOutcome, RmError> {
        Ok(CloseOutcome::Refused {
            reason: reason.into(),
        })
    }
}

impl<A: RmApi> RestartManager for RmSessions<A> {
    fn locking_processes(&self, paths: &[PathBuf]) -> Result<Vec<LockInfo>, RmError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let session = Session::start(&self.api)?;
        for batch in paths.chunks(REGISTER_BATCH) {
            self.api.register_files(session.handle, batch)?;
        }
        self.list_all(session.handle)
    }

    fn request_close(&self, target: UniqueProcess) -> Result<CloseOutcome, RmError> {
        // The two PIDs that are never asked, whatever Windows calls them.
        if let ClosePolicy::InfoOnly { reason, .. } = close_policy(target.pid, AppKind::WindowApp) {
            return Self::refusal(reason);
        }
        match self.api.process_state(target.pid) {
            ProcessState::Gone => return Ok(CloseOutcome::AlreadyExited),
            ProcessState::Unknown => {
                return Self::refusal(
                    "WinKnife could not confirm which program this is, so it did not ask it to close.",
                )
            }
            ProcessState::Running(start) if start != target.start_time => {
                return Self::refusal(
                    "That process has exited and its number now belongs to a different program. Check again.",
                )
            }
            ProcessState::Running(_) => {}
        }

        {
            let session = Session::start(&self.api)?;
            self.api.register_process(session.handle, target)?;
            let listed = self.list_all(session.handle)?;
            let Some(info) = listed.iter().find(|info| info.process == target) else {
                return Self::refusal(
                    "Windows no longer lists this program, so it was not asked to close. Check again.",
                );
            };
            let policy = close_policy(info.process.pid, info.kind);
            if !policy.is_closable() {
                return Self::refusal(policy.reason());
            }
            match self.api.shutdown(session.handle) {
                Ok(()) => {}
                // A build that may not reach the OS says so; it is not a
                // "still running" answer.
                Err(RmError::NotArmed) => return Err(RmError::NotArmed),
                // The program declined or did not answer in time. Whether it
                // is still running is checked below either way.
                Err(err) => {
                    tracing::info!(pid = target.pid, %err, "polite close was not completed")
                }
            }
        }

        Ok(match self.api.process_state(target.pid) {
            ProcessState::Running(start) if start == target.start_time => {
                CloseOutcome::StillRunning
            }
            // Cannot tell: the cautious answer is that it may still be there.
            ProcessState::Unknown => CloseOutcome::StillRunning,
            ProcessState::Running(_) | ProcessState::Gone => CloseOutcome::Closed,
        })
    }

    fn exe_path(&self, pid: u32) -> Option<PathBuf> {
        self.api.exe_path(pid)
    }
}

/// The Restart Manager that talks to Windows.
///
/// Queries work in any build; the polite close refuses unless the composition
/// root armed real OS effects. Both give up after a bounded wait
/// ([`QUERY_TIMEOUT`], [`CLOSE_TIMEOUT`]).
pub fn real_restart_manager() -> Arc<dyn RestartManager> {
    Arc::new(TimedRestartManager::new(
        Arc::new(RmSessions::new(SystemRmApi)),
        QUERY_TIMEOUT,
        CLOSE_TIMEOUT,
    ))
}

/// The real `rstrtmgr.dll` calls.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRmApi;

/// Convert a path to a NUL-terminated UTF-16 string.
#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// UTF-16 buffer up to its first NUL.
#[cfg(windows)]
fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[cfg(windows)]
fn check(
    call: &'static str,
    result: windows::Win32::Foundation::WIN32_ERROR,
) -> Result<(), RmError> {
    if result.0 == 0 {
        Ok(())
    } else {
        Err(RmError::Win32 {
            call,
            code: result.0,
        })
    }
}

#[cfg(windows)]
impl RmApi for SystemRmApi {
    fn start_session(&self) -> Result<u32, RmError> {
        use windows::core::PWSTR;
        use windows::Win32::System::RestartManager::{RmStartSession, CCH_RM_SESSION_KEY};

        let mut handle = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: `key` has the CCH_RM_SESSION_KEY + 1 characters the API
        // requires, and both pointers outlive the call.
        let result = unsafe { RmStartSession(&mut handle, None, PWSTR(key.as_mut_ptr())) };
        check("RmStartSession", result)?;
        Ok(handle)
    }

    fn end_session(&self, session: u32) {
        use windows::Win32::System::RestartManager::RmEndSession;

        // SAFETY: `session` came from a successful RmStartSession and is ended
        // once, by the guard.
        let result = unsafe { RmEndSession(session) };
        if result.0 != 0 {
            tracing::warn!(code = result.0, "RmEndSession failed");
        }
    }

    fn register_files(&self, session: u32, files: &[PathBuf]) -> Result<(), RmError> {
        use windows::core::PCWSTR;
        use windows::Win32::System::RestartManager::RmRegisterResources;

        let strings: Vec<Vec<u16>> = files.iter().map(|path| wide(path)).collect();
        let pointers: Vec<PCWSTR> = strings.iter().map(|s| PCWSTR(s.as_ptr())).collect();
        // SAFETY: every pointer points into `strings`, which outlives the call;
        // each string is NUL-terminated.
        let result = unsafe { RmRegisterResources(session, Some(&pointers), None, None) };
        check("RmRegisterResources", result)
    }

    fn register_process(&self, session: u32, process: UniqueProcess) -> Result<(), RmError> {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::RestartManager::{RmRegisterResources, RM_UNIQUE_PROCESS};

        let unique = RM_UNIQUE_PROCESS {
            dwProcessId: process.pid,
            ProcessStartTime: FILETIME {
                // Splitting the u64 back into its halves; truncation intended.
                dwLowDateTime: (process.start_time & 0xFFFF_FFFF) as u32,
                dwHighDateTime: (process.start_time >> 32) as u32,
            },
        };
        // SAFETY: one plain struct, borrowed for the call.
        let result = unsafe { RmRegisterResources(session, None, Some(&[unique]), None) };
        check("RmRegisterResources", result)
    }

    fn get_list(&self, session: u32, capacity: usize) -> Result<ListAttempt, RmError> {
        use windows::Win32::Foundation::ERROR_MORE_DATA;
        use windows::Win32::System::RestartManager::{RmGetList, RM_PROCESS_INFO};

        let mut buffer = vec![RM_PROCESS_INFO::default(); capacity];
        let mut needed = 0u32;
        let mut count = u32::try_from(capacity).unwrap_or(u32::MAX);
        let mut reasons = 0u32;
        let array = if capacity == 0 {
            None
        } else {
            Some(buffer.as_mut_ptr())
        };
        // SAFETY: `count` is the length of `buffer`, which outlives the call.
        let result = unsafe { RmGetList(session, &mut needed, &mut count, array, &mut reasons) };
        if result == ERROR_MORE_DATA {
            return Ok(ListAttempt::MoreData {
                needed: needed as usize,
            });
        }
        check("RmGetList", result)?;

        let filled = (count as usize).min(buffer.len());
        let list = buffer[..filled]
            .iter()
            .map(|info| {
                let start = u64::from(info.Process.ProcessStartTime.dwHighDateTime) << 32
                    | u64::from(info.Process.ProcessStartTime.dwLowDateTime);
                let service = from_wide(&info.strServiceShortName);
                LockInfo {
                    process: UniqueProcess {
                        pid: info.Process.dwProcessId,
                        start_time: start,
                    },
                    app_name: from_wide(&info.strAppName),
                    service_name: (!service.is_empty()).then_some(service),
                    kind: AppKind::from_rm(info.ApplicationType.0),
                }
            })
            .collect();
        Ok(ListAttempt::Complete(list))
    }

    fn shutdown(&self, session: u32) -> Result<(), RmError> {
        use windows::Win32::System::RestartManager::RmShutdown;

        if !super::real_os_effects_armed() {
            return Err(RmError::NotArmed);
        }
        // Flags 0: ask, never force. No status callback.
        // SAFETY: `session` is live for the duration of the call.
        let result = unsafe { RmShutdown(session, 0, None) };
        check("RmShutdown", result)
    }

    fn process_state(&self, pid: u32) -> ProcessState {
        win_process::state(pid)
    }

    fn exe_path(&self, pid: u32) -> Option<PathBuf> {
        win_process::exe_path(pid)
    }
}

#[cfg(not(windows))]
impl RmApi for SystemRmApi {
    fn start_session(&self) -> Result<u32, RmError> {
        Err(RmError::Unsupported)
    }
    fn end_session(&self, _session: u32) {}
    fn register_files(&self, _session: u32, _files: &[PathBuf]) -> Result<(), RmError> {
        Err(RmError::Unsupported)
    }
    fn register_process(&self, _session: u32, _process: UniqueProcess) -> Result<(), RmError> {
        Err(RmError::Unsupported)
    }
    fn get_list(&self, _session: u32, _capacity: usize) -> Result<ListAttempt, RmError> {
        Err(RmError::Unsupported)
    }
    fn shutdown(&self, _session: u32) -> Result<(), RmError> {
        Err(RmError::Unsupported)
    }
    fn process_state(&self, _pid: u32) -> ProcessState {
        ProcessState::Unknown
    }
    fn exe_path(&self, _pid: u32) -> Option<PathBuf> {
        None
    }
}

/// Read-only process lookups by PID.
#[cfg(windows)]
mod win_process {
    use std::path::PathBuf;

    use windows::core::PWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, STILL_ACTIVE,
    };
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::ProcessState;

    /// Longest path `QueryFullProcessImageNameW` can return.
    const MAX_IMAGE_PATH: usize = 32_768;

    /// A process handle closed on drop. Query rights only.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful OpenProcess.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    fn open(pid: u32) -> Result<Handle, windows::core::Error> {
        // SAFETY: plain call; the handle is owned by `Handle`.
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.map(Handle)
    }

    pub(super) fn state(pid: u32) -> ProcessState {
        let handle = match open(pid) {
            Ok(handle) => handle,
            // OpenProcess answers "invalid parameter" for a PID nobody has.
            Err(err) if err.code() == ERROR_INVALID_PARAMETER.to_hresult() => {
                return ProcessState::Gone
            }
            Err(_) => return ProcessState::Unknown,
        };
        let mut exit_code = 0u32;
        // SAFETY: valid handle, valid out pointer.
        if unsafe { GetExitCodeProcess(handle.0, &mut exit_code) }.is_err() {
            return ProcessState::Unknown;
        }
        // An exited process can still be opened while someone holds a handle.
        if exit_code != STILL_ACTIVE.0 as u32 {
            return ProcessState::Gone;
        }
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: valid handle, four valid out pointers.
        if unsafe { GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user) }
            .is_err()
        {
            return ProcessState::Unknown;
        }
        ProcessState::Running(
            u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime),
        )
    }

    pub(super) fn exe_path(pid: u32) -> Option<PathBuf> {
        let handle = open(pid).ok()?;
        let mut buffer = vec![0u16; MAX_IMAGE_PATH];
        let mut len = u32::try_from(buffer.len()).ok()?;
        // SAFETY: `len` is the buffer length; the buffer outlives the call.
        unsafe {
            QueryFullProcessImageNameW(
                handle.0,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut len,
            )
        }
        .ok()?;
        let end = (len as usize).min(buffer.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
    }
}

/// A scripted [`RmApi`] for unit tests.
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::{ListAttempt, LockInfo, ProcessState, RmApi, RmError, UniqueProcess};

    /// Files and processes registered in one session.
    pub type Registered = (Vec<PathBuf>, Vec<UniqueProcess>);

    /// What the fake remembers about the calls it saw.
    #[derive(Debug, Default)]
    pub struct Calls {
        pub started: u32,
        pub ended: Vec<u32>,
        pub file_batches: Vec<usize>,
        pub registered_processes: Vec<UniqueProcess>,
        pub get_list_capacities: Vec<usize>,
        pub shutdowns: u32,
    }

    /// The fake's world: who holds what, who is running, how it misbehaves.
    #[derive(Debug, Default)]
    pub struct FakeRmApi {
        /// Holder → files it has open.
        pub holders: Vec<(LockInfo, Vec<PathBuf>)>,
        /// Running processes.
        pub running: Mutex<HashMap<u32, ProcessState>>,
        /// Answer `MoreData` this many times before the real list.
        pub more_data_rounds: Mutex<usize>,
        /// Fail every file registration.
        pub fail_register: bool,
        /// Whether a polite shutdown makes the process exit.
        pub shutdown_closes: bool,
        /// Exe paths by PID.
        pub exe_paths: HashMap<u32, PathBuf>,
        pub calls: Mutex<Calls>,
        pub registered: Mutex<HashMap<u32, Registered>>,
    }

    impl FakeRmApi {
        /// A fake where `holders` hold files and are running.
        pub fn with_holders(holders: Vec<(LockInfo, Vec<PathBuf>)>) -> Self {
            let running = holders
                .iter()
                .map(|(info, _)| {
                    (
                        info.process.pid,
                        ProcessState::Running(info.process.start_time),
                    )
                })
                .collect();
            Self {
                holders,
                running: Mutex::new(running),
                ..Self::default()
            }
        }

        pub fn calls(&self) -> std::sync::MutexGuard<'_, Calls> {
            self.calls.lock().expect("calls")
        }

        fn holds(files: &[PathBuf], wanted: &[PathBuf]) -> bool {
            files
                .iter()
                .any(|f| wanted.iter().any(|w| Path::new(w) == f))
        }
    }

    impl RmApi for FakeRmApi {
        fn start_session(&self) -> Result<u32, RmError> {
            let mut calls = self.calls();
            calls.started += 1;
            Ok(calls.started)
        }

        fn end_session(&self, session: u32) {
            self.calls().ended.push(session);
        }

        fn register_files(&self, session: u32, files: &[PathBuf]) -> Result<(), RmError> {
            self.calls().file_batches.push(files.len());
            if self.fail_register {
                return Err(RmError::Win32 {
                    call: "RmRegisterResources",
                    code: 5,
                });
            }
            let mut registered = self.registered.lock().expect("registered");
            registered
                .entry(session)
                .or_default()
                .0
                .extend_from_slice(files);
            Ok(())
        }

        fn register_process(&self, session: u32, process: UniqueProcess) -> Result<(), RmError> {
            self.calls().registered_processes.push(process);
            let mut registered = self.registered.lock().expect("registered");
            registered.entry(session).or_default().1.push(process);
            Ok(())
        }

        fn get_list(&self, session: u32, capacity: usize) -> Result<ListAttempt, RmError> {
            self.calls().get_list_capacities.push(capacity);
            let registered = self.registered.lock().expect("registered");
            let (files, processes) = registered.get(&session).cloned().unwrap_or_default();
            let running = self.running.lock().expect("running");
            let list: Vec<LockInfo> = self
                .holders
                .iter()
                .filter(|(info, held)| {
                    running.get(&info.process.pid)
                        == Some(&ProcessState::Running(info.process.start_time))
                        && (Self::holds(held, &files) || processes.contains(&info.process))
                })
                .map(|(info, _)| info.clone())
                .collect();
            let mut rounds = self.more_data_rounds.lock().expect("rounds");
            if *rounds > 0 || list.len() > capacity {
                *rounds = rounds.saturating_sub(1);
                return Ok(ListAttempt::MoreData {
                    needed: list.len().max(capacity + 1),
                });
            }
            Ok(ListAttempt::Complete(list))
        }

        fn shutdown(&self, session: u32) -> Result<(), RmError> {
            self.calls().shutdowns += 1;
            if !self.shutdown_closes {
                return Err(RmError::Win32 {
                    call: "RmShutdown",
                    code: 351,
                });
            }
            let registered = self.registered.lock().expect("registered");
            let processes = registered
                .get(&session)
                .map(|r| r.1.clone())
                .unwrap_or_default();
            let mut running = self.running.lock().expect("running");
            for process in processes {
                running.insert(process.pid, ProcessState::Gone);
            }
            Ok(())
        }

        fn process_state(&self, pid: u32) -> ProcessState {
            self.running
                .lock()
                .expect("running")
                .get(&pid)
                .copied()
                .unwrap_or(ProcessState::Gone)
        }

        fn exe_path(&self, pid: u32) -> Option<PathBuf> {
            self.exe_paths.get(&pid).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeRmApi;
    use super::*;

    fn holder(pid: u32, kind: AppKind) -> LockInfo {
        LockInfo {
            process: UniqueProcess {
                pid,
                start_time: u64::from(pid) * 1_000,
            },
            app_name: format!("app{pid}"),
            service_name: None,
            kind,
        }
    }

    fn rm_with(holders: Vec<(LockInfo, Vec<PathBuf>)>) -> RmSessions<FakeRmApi> {
        RmSessions::new(FakeRmApi::with_holders(holders))
    }

    fn assert_every_session_ended(rm: &RmSessions<FakeRmApi>) {
        let calls = rm.api().calls();
        assert!(calls.started > 0, "no session was started");
        let mut ended = calls.ended.clone();
        ended.sort_unstable();
        assert_eq!(ended, (1..=calls.started).collect::<Vec<_>>());
    }

    #[test]
    fn the_classification_table_decides_who_can_be_asked() {
        let table = [
            (AppKind::WindowApp, true, "window"),
            (AppKind::Console, true, "console"),
            (AppKind::Service, false, "service"),
            (AppKind::Explorer, false, "explorer"),
            (AppKind::Critical, false, "critical"),
            (AppKind::Unknown, false, "unknown"),
        ];
        for (kind, closable, code) in table {
            let policy = close_policy(1234, kind);
            assert_eq!(policy.is_closable(), closable, "{kind:?}");
            assert_eq!(policy.code(), code);
            assert!(!policy.reason().is_empty());
        }
        assert!(!close_policy(SYSTEM_PID, AppKind::WindowApp).is_closable());
        assert!(!close_policy(std::process::id(), AppKind::WindowApp).is_closable());
    }

    #[test]
    fn info_only_reasons_say_what_to_do_instead() {
        assert_eq!(
            close_policy(10, AppKind::Service).reason(),
            "Windows service — stop it from Services if you are sure."
        );
        assert_eq!(
            close_policy(10, AppKind::Explorer).reason(),
            "Explorer — restarting it is safer than closing it; WinKnife does not do this for you."
        );
    }

    #[test]
    fn raw_app_types_map_to_kinds() {
        assert_eq!(AppKind::from_rm(1), AppKind::WindowApp);
        assert_eq!(AppKind::from_rm(2), AppKind::WindowApp);
        assert_eq!(AppKind::from_rm(3), AppKind::Service);
        assert_eq!(AppKind::from_rm(4), AppKind::Explorer);
        assert_eq!(AppKind::from_rm(5), AppKind::Console);
        assert_eq!(AppKind::from_rm(1000), AppKind::Critical);
        assert_eq!(AppKind::from_rm(0), AppKind::Unknown);
        assert_eq!(AppKind::from_rm(77), AppKind::Unknown);
    }

    #[test]
    fn files_are_registered_in_batches_of_500() {
        let rm = rm_with(Vec::new());
        let files: Vec<PathBuf> = (0..1_234)
            .map(|i| PathBuf::from(format!("C:\\f{i}")))
            .collect();
        rm.locking_processes(&files).expect("query");
        assert_eq!(rm.api().calls().file_batches, vec![500, 500, 234]);
        assert_every_session_ended(&rm);
    }

    #[test]
    fn the_session_is_ended_when_registration_fails() {
        let rm = RmSessions::new(FakeRmApi {
            fail_register: true,
            ..FakeRmApi::default()
        });
        let err = rm
            .locking_processes(&[PathBuf::from("C:\\a")])
            .expect_err("fails");
        assert!(matches!(err, RmError::Win32 { .. }));
        assert_every_session_ended(&rm);
    }

    #[test]
    fn more_data_is_retried_a_bounded_number_of_times() {
        let a = PathBuf::from("C:\\a");
        let rm = rm_with(vec![(holder(10, AppKind::WindowApp), vec![a.clone()])]);
        *rm.api().more_data_rounds.lock().expect("rounds") = MORE_DATA_RETRIES;
        let list = rm
            .locking_processes(std::slice::from_ref(&a))
            .expect("last retry succeeds");
        assert_eq!(list.len(), 1);
        assert_eq!(
            rm.api().calls().get_list_capacities.len(),
            MORE_DATA_RETRIES + 1
        );

        let rm = rm_with(vec![(holder(10, AppKind::WindowApp), vec![a.clone()])]);
        *rm.api().more_data_rounds.lock().expect("rounds") = MORE_DATA_RETRIES + 1;
        let err = rm.locking_processes(&[a]).expect_err("gives up");
        assert!(matches!(err, RmError::ListKeptGrowing));
        assert_eq!(
            rm.api().calls().get_list_capacities.len(),
            MORE_DATA_RETRIES + 1
        );
        assert_every_session_ended(&rm);
    }

    #[test]
    fn an_empty_query_starts_no_session() {
        let rm = rm_with(Vec::new());
        assert!(rm.locking_processes(&[]).expect("empty").is_empty());
        assert_eq!(rm.api().calls().started, 0);
    }

    #[test]
    fn a_window_app_is_asked_in_a_fresh_session_with_only_itself() {
        let a = PathBuf::from("C:\\a");
        let word = holder(10, AppKind::WindowApp);
        let mut api = FakeRmApi::with_holders(vec![
            (word.clone(), vec![a.clone()]),
            (holder(11, AppKind::WindowApp), vec![a]),
        ]);
        api.shutdown_closes = true;
        let rm = RmSessions::new(api);

        let outcome = rm.request_close(word.process).expect("close");
        assert_eq!(outcome, CloseOutcome::Closed);
        let calls = rm.api().calls();
        assert_eq!(calls.registered_processes, vec![word.process]);
        assert!(
            calls.file_batches.is_empty(),
            "no files in the close session"
        );
        assert_eq!(calls.shutdowns, 1);
        drop(calls);
        assert_every_session_ended(&rm);
    }

    #[test]
    fn a_program_that_declines_is_reported_still_running() {
        let word = holder(10, AppKind::WindowApp);
        let rm = rm_with(vec![(word.clone(), vec![PathBuf::from("C:\\a")])]);
        assert_eq!(
            rm.request_close(word.process).expect("asked"),
            CloseOutcome::StillRunning
        );
        assert_every_session_ended(&rm);
    }

    #[test]
    fn a_reused_pid_is_refused_before_anything_is_asked() {
        let word = holder(10, AppKind::WindowApp);
        let rm = rm_with(vec![(word.clone(), vec![PathBuf::from("C:\\a")])]);
        let stale = UniqueProcess {
            pid: word.process.pid,
            start_time: word.process.start_time + 1,
        };
        let outcome = rm.request_close(stale).expect("answered");
        assert!(
            matches!(outcome, CloseOutcome::Refused { .. }),
            "{outcome:?}"
        );
        let calls = rm.api().calls();
        assert_eq!(calls.started, 0);
        assert_eq!(calls.shutdowns, 0);
    }

    #[test]
    fn info_only_kinds_are_refused_server_side() {
        for kind in [
            AppKind::Service,
            AppKind::Explorer,
            AppKind::Critical,
            AppKind::Unknown,
        ] {
            let info = holder(20, kind);
            let mut api =
                FakeRmApi::with_holders(vec![(info.clone(), vec![PathBuf::from("C:\\a")])]);
            api.shutdown_closes = true;
            let rm = RmSessions::new(api);
            let outcome = rm.request_close(info.process).expect("answered");
            assert!(matches!(outcome, CloseOutcome::Refused { .. }), "{kind:?}");
            assert_eq!(rm.api().calls().shutdowns, 0, "{kind:?}");
            assert_every_session_ended(&rm);
        }
    }

    #[test]
    fn system_and_self_are_refused_without_a_session() {
        for pid in [SYSTEM_PID, std::process::id()] {
            let rm = rm_with(Vec::new());
            let outcome = rm
                .request_close(UniqueProcess { pid, start_time: 1 })
                .expect("answered");
            assert!(matches!(outcome, CloseOutcome::Refused { .. }));
            assert_eq!(rm.api().calls().started, 0);
        }
    }

    #[test]
    fn a_process_already_gone_is_not_asked() {
        let rm = rm_with(Vec::new());
        let outcome = rm
            .request_close(UniqueProcess {
                pid: 999,
                start_time: 1,
            })
            .expect("answered");
        assert_eq!(outcome, CloseOutcome::AlreadyExited);
        assert_eq!(rm.api().calls().started, 0);
    }

    /// The real wrapper can never ask a real program to close from a test.
    #[cfg(windows)]
    #[test]
    fn the_real_shutdown_refuses_in_a_test_build() {
        assert!(!super::super::real_os_effects_armed());
        assert!(matches!(SystemRmApi.shutdown(0), Err(RmError::NotArmed)));
    }

    /// The real query runs read-only in any build.
    #[cfg(windows)]
    #[test]
    fn the_real_process_lookup_finds_this_test() {
        let pid = std::process::id();
        assert!(matches!(
            SystemRmApi.process_state(pid),
            ProcessState::Running(_)
        ));
        assert!(SystemRmApi.exe_path(pid).is_some());
    }
}
