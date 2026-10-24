//! Creating a Windows restore point (module spec 09).
//!
//! Two processes take part. The app (unelevated) re-launches its own exe with
//! [`CREATE_RESTORE_POINT_FLAG`] through `ShellExecuteExW` verb `runas`, which
//! is what shows the UAC prompt, and waits up to [`HELPER_TIMEOUT`] for it. The
//! elevated copy never builds the app: `lib.rs::run()` sees the flag first,
//! calls [`create_in_helper`] and exits with one of the `EXIT_*` codes below.
//! Those codes are the whole protocol between the two, so they live here,
//! shared by both sides.
//!
//! Nothing here changes System Protection settings or
//! `SystemRestorePointCreationFrequency` (spec: read only). The only write is
//! the restore point itself, made by Windows through `SRSetRestorePointW`.
//!
//! The parent side is the trait [`RestorePointPlatform`], so the exit-code →
//! result mapping is unit-tested against [`FakeRestorePointPlatform`]. The real
//! implementation refuses until [`real_os_effects_armed`](super::real_os_effects_armed),
//! like every other call that reaches the OS for real.

use std::sync::Mutex;
use std::time::Duration;

use crate::core::error::{CoreError, Result};

/// Command-line flag that turns the exe into the elevated helper.
pub const CREATE_RESTORE_POINT_FLAG: &str = "--create-restore-point";

/// How long the app waits for the helper (UAC prompt included).
pub const HELPER_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The helper created a restore point: the newest sequence number moved.
pub const EXIT_CREATED: u32 = 0;
/// Windows accepted the call but made no new point: the newest sequence number
/// did not move. Windows allows one per 24 hours by default.
pub const EXIT_SKIPPED: u32 = 10;
/// Windows accepted the call, but the helper could not read the restore point
/// list to tell whether a new one exists.
pub const EXIT_MAYBE_SKIPPED: u32 = 11;
/// System Protection is off (`SRSetRestorePointW` → `ERROR_SERVICE_DISABLED`).
pub const EXIT_PROTECTION_OFF: u32 = 20;
/// Any other failure: this bit plus the Win32 error in the low 16 bits. Bit 29
/// is the "customer" bit, so the result never collides with a system code or
/// with the small codes above, and it stays a positive `i32` for
/// `std::process::exit`.
pub const EXIT_FAILED_BIT: u32 = 0x2000_0000;

/// Win32 error codes fit 16 bits.
const WIN32_CODE_MASK: u32 = 0xFFFF;

/// `ERROR_SERVICE_DISABLED`: what `SRSetRestorePointW` reports when System
/// Protection is turned off.
pub const WIN32_SERVICE_DISABLED: u32 = 1058;
/// `ERROR_CANCELLED`: the user said No at the UAC prompt.
pub const WIN32_CANCELLED: u32 = 1223;

/// The exit code for a failure with `win32_code`.
pub fn failed_exit_code(win32_code: u32) -> u32 {
    EXIT_FAILED_BIT | (win32_code & WIN32_CODE_MASK)
}

/// The Win32 error inside a failure exit code, or `None` for any other code.
pub fn failure_from_exit_code(exit_code: u32) -> Option<u32> {
    (exit_code & !WIN32_CODE_MASK == EXIT_FAILED_BIT).then_some(exit_code & WIN32_CODE_MASK)
}

/// What the helper's restore-point list looked like: `Ok(None)` is a machine
/// with no restore points, `Err(())` a list that could not be read.
pub type NewestSequence = std::result::Result<Option<u32>, ()>;

/// The helper's verdict from the newest sequence number before and after the
/// call. Pure, so the 24-hour detection is tested without Windows.
pub fn classify_sequences(before: NewestSequence, after: NewestSequence) -> u32 {
    match (before, after) {
        (Ok(before), Ok(Some(after))) if before.is_none_or(|before| after > before) => EXIT_CREATED,
        (Ok(before), Ok(after)) if before == after => EXIT_SKIPPED,
        // Fewer points than before, or a list that could not be read: the
        // honest answer is "cannot tell".
        _ => EXIT_MAYBE_SKIPPED,
    }
}

/// How a run of the elevated helper ended, from the app's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperRun {
    /// The helper ran and exited with this code.
    Exited(u32),
    /// The user answered No at the UAC prompt.
    Declined,
    /// Still running after [`HELPER_TIMEOUT`]. It is left alone, not killed.
    TimedOut,
    /// The helper could not be started; the Win32 error.
    LaunchFailed(u32),
}

/// The OS surface the restore-point module needs, app side.
pub trait RestorePointPlatform: Send + Sync {
    /// Re-launch this exe elevated as the helper and wait for it.
    ///
    /// # Errors
    /// Returns an error when this build is not allowed to reach the OS.
    fn run_helper(&self, description: &str) -> Result<HelperRun>;

    /// Open Windows' System Protection settings. Only ever on a button click.
    ///
    /// # Errors
    /// Returns an error when the window could not be opened, or when this build
    /// is not allowed to reach the OS.
    fn open_protection_settings(&self) -> Result<()>;
}

/// The real implementation.
#[derive(Debug, Default)]
pub struct SystemRestorePoints;

impl RestorePointPlatform for SystemRestorePoints {
    fn run_helper(&self, description: &str) -> Result<HelperRun> {
        if !super::real_os_effects_armed() {
            return Err(CoreError::RestorePoint(
                "this build must never start an elevated helper".to_owned(),
            ));
        }
        // The description was sanitised: no quotes, no backslashes, no control
        // characters, so wrapping it in quotes is the whole escaping job.
        run_elevated(&format!("{CREATE_RESTORE_POINT_FLAG} \"{description}\""))
    }

    fn open_protection_settings(&self) -> Result<()> {
        if !super::real_os_effects_armed() {
            return Err(CoreError::RestorePoint(
                "this build must never open System Protection settings".to_owned(),
            ));
        }
        sys::open_protection_settings()
    }
}

/// Re-launch this exe through `runas` (the UAC prompt) with `parameters` and
/// wait up to [`HELPER_TIMEOUT`] for it. Shared by every elevated helper; the
/// caller checks [`real_os_effects_armed`](super::real_os_effects_armed) first.
///
/// # Errors
/// Returns an error when WinKnife's own exe cannot be located.
pub(crate) fn run_elevated(parameters: &str) -> Result<HelperRun> {
    sys::run_elevated(parameters)
}

/// The elevated helper's whole job: ask Windows for a restore point and say
/// what happened as an exit code. Never panics on a Windows failure.
pub fn create_in_helper(description: &str) -> u32 {
    if !super::real_os_effects_armed() {
        return failed_exit_code(WIN32_ACCESS_DENIED);
    }
    sys::create_in_helper(description)
}

/// `ERROR_ACCESS_DENIED`, for a helper started from a build that may not act.
const WIN32_ACCESS_DENIED: u32 = 5;

/* -------------------------------------------------------------------------
 * The fake
 * ---------------------------------------------------------------------- */

/// A scripted platform for tests: returns the configured run and records every
/// description it was handed.
#[derive(Debug)]
pub struct FakeRestorePointPlatform {
    run: HelperRun,
    descriptions: Mutex<Vec<String>>,
    settings_opened: Mutex<u32>,
}

impl FakeRestorePointPlatform {
    /// A fake whose helper always ends with `run`.
    pub fn new(run: HelperRun) -> Self {
        Self {
            run,
            descriptions: Mutex::new(Vec::new()),
            settings_opened: Mutex::new(0),
        }
    }

    /// Every description passed to [`RestorePointPlatform::run_helper`].
    pub fn descriptions(&self) -> Vec<String> {
        self.descriptions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// How many times the settings window was "opened".
    pub fn settings_opened(&self) -> u32 {
        *self
            .settings_opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl RestorePointPlatform for FakeRestorePointPlatform {
    fn run_helper(&self, description: &str) -> Result<HelperRun> {
        self.descriptions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(description.to_owned());
        Ok(self.run)
    }

    fn open_protection_settings(&self) -> Result<()> {
        *self
            .settings_opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Ok(())
    }
}

/* -------------------------------------------------------------------------
 * Windows
 * ---------------------------------------------------------------------- */

#[cfg(windows)]
mod sys {
    use super::{
        classify_sequences, failed_exit_code, HelperRun, NewestSequence, EXIT_PROTECTION_OFF,
        HELPER_TIMEOUT, WIN32_CANCELLED, WIN32_SERVICE_DISABLED,
    };
    use crate::core::error::{CoreError, Result};
    use windows::core::{w, BSTR, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoInitializeSecurity, CoSetProxyBlanket, CoUninitialize,
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
        RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Restore::{
        SRSetRestorePointW, BEGIN_SYSTEM_CHANGE, END_SYSTEM_CHANGE, MODIFY_SETTINGS,
        RESTOREPOINTINFOW, STATEMGRSTATUS,
    };
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::System::Wmi::{
        IWbemLocator, WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
        WBEM_INFINITE,
    };
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

    /// `RPC_C_AUTHN_WINNT` / `RPC_C_AUTHZ_NONE` (the `Rpc` feature is not
    /// worth pulling in for two integers).
    const RPC_C_AUTHN_WINNT: u32 = 10;
    const RPC_C_AUTHZ_NONE: u32 = 0;
    /// `RPC_E_CHANGED_MODE` / `RPC_E_TOO_LATE`: COM was already set up by
    /// someone else in this process; both are fine to work under.
    const RPC_E_CHANGED_MODE: u32 = 0x8001_0106;
    const RPC_E_TOO_LATE: u32 = 0x8001_0119;
    /// Restore points live in this WMI namespace and class.
    const WMI_NAMESPACE: &str = "ROOT\\DEFAULT";
    const WMI_QUERY: &str = "SELECT SequenceNumber FROM SystemRestore";

    /// The Win32 error inside a `HRESULT_FROM_WIN32` value, or the whole
    /// `HRESULT` when it is not one.
    fn win32_code(error: &windows::core::Error) -> u32 {
        let hr = error.code().0 as u32;
        if hr & 0xFFFF_0000 == 0x8007_0000 {
            hr & 0xFFFF
        } else {
            hr
        }
    }

    pub(super) fn run_elevated(parameters: &str) -> Result<HelperRun> {
        let exe = std::env::current_exe()?;
        let file = HSTRING::from(exe.as_os_str());
        let parameters = HSTRING::from(parameters);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or(u32::MAX),
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: every pointer in `info` outlives the call.
        if let Err(err) = unsafe { ShellExecuteExW(&mut info) } {
            let code = win32_code(&err);
            return Ok(if code == WIN32_CANCELLED {
                HelperRun::Declined
            } else {
                HelperRun::LaunchFailed(code)
            });
        }
        if info.hProcess.is_invalid() {
            return Ok(HelperRun::LaunchFailed(0));
        }
        let run = wait_for(info.hProcess);
        // SAFETY: the handle was opened for us by SEE_MASK_NOCLOSEPROCESS.
        let _ = unsafe { CloseHandle(info.hProcess) };
        Ok(run)
    }

    fn wait_for(process: HANDLE) -> HelperRun {
        let millis = u32::try_from(HELPER_TIMEOUT.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: a live process handle.
        let waited = unsafe { WaitForSingleObject(process, millis) };
        if waited == WAIT_TIMEOUT {
            // Left running: it is creating a restore point, and killing that
            // halfway is worse than waiting it out (no force-kill, phase 5).
            return HelperRun::TimedOut;
        }
        if waited != WAIT_OBJECT_0 {
            return HelperRun::LaunchFailed(waited.0);
        }
        let mut code = 0_u32;
        // SAFETY: a live process handle and a valid out-pointer.
        match unsafe { GetExitCodeProcess(process, &mut code) } {
            Ok(()) => HelperRun::Exited(code),
            Err(err) => HelperRun::LaunchFailed(win32_code(&err)),
        }
    }

    pub(super) fn open_protection_settings() -> Result<()> {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let exe = std::path::Path::new(&root)
            .join("System32")
            .join("SystemPropertiesProtection.exe");
        let file = HSTRING::from(exe.as_os_str());
        let mut info = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or(u32::MAX),
            fMask: SEE_MASK_NOASYNC,
            lpFile: PCWSTR(file.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: every pointer in `info` outlives the call. The shell, not
        // `CreateProcess`, so the window's own elevation request is honoured.
        match unsafe { ShellExecuteExW(&mut info) } {
            Ok(()) => Ok(()),
            // No at that window's own UAC prompt: nothing went wrong.
            Err(err) if win32_code(&err) == WIN32_CANCELLED => Ok(()),
            Err(err) => Err(CoreError::RestorePoint(format!(
                "could not open System Protection settings: {err}"
            ))),
        }
    }

    /// COM for the helper's own thread; balanced on drop when we started it.
    struct Com {
        owned: bool,
    }

    impl Com {
        fn init() -> std::result::Result<Self, u32> {
            // SAFETY: initialising the calling thread's COM apartment.
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_ok() {
                return Ok(Self { owned: true });
            }
            if hr.0 as u32 == RPC_E_CHANGED_MODE {
                return Ok(Self { owned: false });
            }
            Err(hr.0 as u32)
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.owned {
                // SAFETY: balancing the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        }
    }

    pub(super) fn create_in_helper(description: &str) -> u32 {
        let com = match Com::init() {
            Ok(com) => Some(com),
            // No COM means no WMI; the restore point itself does not need it,
            // so carry on and report "cannot tell" instead of failing.
            Err(hr) => {
                tracing::warn!(hr = format!("0x{hr:08X}"), "COM unavailable in the helper");
                None
            }
        };
        if com.is_some() {
            init_security();
        }
        let before = if com.is_some() {
            newest_sequence()
        } else {
            Err(())
        };

        // Built locally and moved in whole: the struct is packed, so its
        // fields cannot be borrowed.
        let mut text = [0_u16; 256];
        for (slot, unit) in text.iter_mut().zip(description.encode_utf16().take(255)) {
            *slot = unit;
        }
        let spec = RESTOREPOINTINFOW {
            dwEventType: BEGIN_SYSTEM_CHANGE,
            dwRestorePtType: MODIFY_SETTINGS,
            llSequenceNumber: 0,
            szDescription: text,
        };
        let mut status = STATEMGRSTATUS::default();
        // SAFETY: both structs are fully initialised and live for the call.
        let begun = unsafe { SRSetRestorePointW(&spec, &mut status) }.as_bool();
        let code = status.nStatus.0;
        if !begun {
            return if code == WIN32_SERVICE_DISABLED {
                EXIT_PROTECTION_OFF
            } else {
                failed_exit_code(code)
            };
        }

        let end = RESTOREPOINTINFOW {
            dwEventType: END_SYSTEM_CHANGE,
            dwRestorePtType: MODIFY_SETTINGS,
            llSequenceNumber: status.llSequenceNumber,
            szDescription: [0; 256],
        };
        let mut end_status = STATEMGRSTATUS::default();
        // SAFETY: as above.
        let ended = unsafe { SRSetRestorePointW(&end, &mut end_status) }.as_bool();
        if !ended {
            return failed_exit_code(end_status.nStatus.0);
        }

        let after = if com.is_some() {
            newest_sequence()
        } else {
            Err(())
        };
        drop(com);
        classify_sequences(before, after)
    }

    fn init_security() {
        // SAFETY: process-wide default COM security for the helper, which is
        // a process of its own that does nothing else.
        let result = unsafe {
            CoInitializeSecurity(
                None,
                -1,
                None,
                None,
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
                None,
            )
        };
        if let Err(err) = result {
            if err.code().0 as u32 != RPC_E_TOO_LATE {
                tracing::warn!(error = %err, "COM security was not set in the helper");
            }
        }
    }

    /// The highest restore point sequence number, read-only through WMI.
    fn newest_sequence() -> NewestSequence {
        query_newest().map_err(|err| {
            tracing::warn!(error = %err, "could not read the restore point list");
        })
    }

    fn query_newest() -> windows::core::Result<Option<u32>> {
        // SAFETY: plain COM calls on interfaces created right here.
        unsafe {
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
            let services = locator.ConnectServer(
                &BSTR::from(WMI_NAMESPACE),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None,
            )?;
            CoSetProxyBlanket(
                &services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )?;
            let rows = services.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(WMI_QUERY),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )?;
            let mut newest: Option<u32> = None;
            loop {
                let mut row = [None];
                let mut returned = 0_u32;
                let hr = rows.Next(WBEM_INFINITE, &mut row, &mut returned);
                hr.ok()?;
                let Some(object) = row[0].take().filter(|_| returned == 1) else {
                    break;
                };
                let mut value = VARIANT::default();
                object.Get(w!("SequenceNumber"), 0, &mut value, None, None)?;
                // WMI hands uint32 over as VT_I4.
                let sequence = u32::try_from(&value)
                    .or_else(|_| i32::try_from(&value).map(i32::cast_unsigned))?;
                newest = Some(newest.map_or(sequence, |newest| newest.max(sequence)));
            }
            Ok(newest)
        }
    }
}

#[cfg(not(windows))]
mod sys {
    use super::{failed_exit_code, HelperRun};
    use crate::core::error::{CoreError, Result};

    /// `ERROR_NOT_SUPPORTED`.
    const NOT_SUPPORTED: u32 = 50;

    pub(super) fn run_elevated(_parameters: &str) -> Result<HelperRun> {
        Ok(HelperRun::LaunchFailed(NOT_SUPPORTED))
    }

    pub(super) fn open_protection_settings() -> Result<()> {
        Err(CoreError::RestorePoint(
            "there are no restore points on this platform".to_owned(),
        ))
    }

    pub(super) fn create_in_helper(_description: &str) -> u32 {
        failed_exit_code(NOT_SUPPORTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_highest_sequence_is_a_created_point() {
        assert_eq!(classify_sequences(Ok(Some(7)), Ok(Some(8))), EXIT_CREATED);
        assert_eq!(classify_sequences(Ok(None), Ok(Some(1))), EXIT_CREATED);
    }

    #[test]
    fn an_unchanged_sequence_is_a_skip() {
        assert_eq!(classify_sequences(Ok(Some(7)), Ok(Some(7))), EXIT_SKIPPED);
        assert_eq!(classify_sequences(Ok(None), Ok(None)), EXIT_SKIPPED);
    }

    #[test]
    fn an_unreadable_or_shrunk_list_cannot_tell() {
        assert_eq!(classify_sequences(Err(()), Ok(Some(8))), EXIT_MAYBE_SKIPPED);
        assert_eq!(classify_sequences(Ok(Some(7)), Err(())), EXIT_MAYBE_SKIPPED);
        assert_eq!(
            classify_sequences(Ok(Some(7)), Ok(Some(6))),
            EXIT_MAYBE_SKIPPED
        );
        assert_eq!(
            classify_sequences(Ok(Some(7)), Ok(None)),
            EXIT_MAYBE_SKIPPED
        );
    }

    #[test]
    fn failure_codes_never_collide_with_the_small_codes() {
        for small in [
            EXIT_CREATED,
            EXIT_SKIPPED,
            EXIT_MAYBE_SKIPPED,
            EXIT_PROTECTION_OFF,
        ] {
            assert_eq!(small & EXIT_FAILED_BIT, 0);
        }
        assert_eq!(failed_exit_code(5), 0x2000_0005);
        assert_ne!(failed_exit_code(0), EXIT_CREATED);
        // Fits a positive i32, which is what `std::process::exit` takes.
        assert!(i32::try_from(failed_exit_code(u32::MAX)).is_ok());
        assert_eq!(failure_from_exit_code(failed_exit_code(1058)), Some(1058));
        assert_eq!(failure_from_exit_code(EXIT_SKIPPED), None);
        assert_eq!(failure_from_exit_code(101), None);
    }

    /// A test run must never raise a UAC prompt or open a settings window.
    #[test]
    fn a_test_process_cannot_reach_the_real_platform() {
        assert!(!super::super::real_os_effects_armed());
        let real = SystemRestorePoints;
        assert!(matches!(
            real.run_helper("x"),
            Err(CoreError::RestorePoint(_))
        ));
        assert!(matches!(
            real.open_protection_settings(),
            Err(CoreError::RestorePoint(_))
        ));
        assert_eq!(create_in_helper("x"), failed_exit_code(WIN32_ACCESS_DENIED));
    }
}
