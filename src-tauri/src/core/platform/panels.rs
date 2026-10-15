//! Opening one of Windows' own classic windows (module spec 10).
//!
//! WinKnife changes nothing here: it asks the shell to open `control.exe`,
//! `mmc.exe` or a System32 tool, and Windows does the rest, its own UAC prompt
//! included. Still gated on [`super::real_os_effects_armed`], like
//! [`super::reveal_in_explorer`], so a test can never open a real window.

use std::path::Path;
use std::sync::Mutex;

use crate::core::error::{CoreError, Result};

/// The two calls the legacy-panels module needs.
pub trait PanelLauncher: Send + Sync {
    /// Whether `path` is a file on this machine. Read-only.
    fn exists(&self, path: &Path) -> bool;

    /// Open `file` through the shell, with `argument` (a path) as its only
    /// parameter. Saying No at a UAC prompt is `Ok`.
    ///
    /// # Errors
    /// Returns an error when the shell could not start the window, or when
    /// this build may not reach the OS.
    fn open(&self, file: &Path, argument: Option<&Path>) -> Result<()>;
}

/// The real implementation.
#[derive(Debug, Default)]
pub struct SystemPanelLauncher;

impl PanelLauncher for SystemPanelLauncher {
    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn open(&self, file: &Path, argument: Option<&Path>) -> Result<()> {
        if !super::real_os_effects_armed() {
            return Err(CoreError::LegacyPanel(
                "this build must never open a Windows panel".to_owned(),
            ));
        }
        sys::open(file, argument)
    }
}

/// One call the fake saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelLaunch {
    /// The program that was asked to start.
    pub file: std::path::PathBuf,
    /// Its single parameter, if any.
    pub argument: Option<std::path::PathBuf>,
}

/// A fake for tests: every path exists unless listed as missing, and every
/// launch is recorded instead of performed.
#[derive(Debug, Default)]
pub struct FakePanelLauncher {
    missing: Vec<std::path::PathBuf>,
    launches: Mutex<Vec<PanelLaunch>>,
}

impl FakePanelLauncher {
    /// A fake where these paths are missing and everything else exists.
    pub fn with_missing(missing: impl IntoIterator<Item = std::path::PathBuf>) -> Self {
        Self {
            missing: missing.into_iter().collect(),
            launches: Mutex::new(Vec::new()),
        }
    }

    /// Every launch so far, in order.
    pub fn launches(&self) -> Vec<PanelLaunch> {
        self.launches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl PanelLauncher for FakePanelLauncher {
    fn exists(&self, path: &Path) -> bool {
        !self.missing.iter().any(|missing| missing == path)
    }

    fn open(&self, file: &Path, argument: Option<&Path>) -> Result<()> {
        self.launches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(PanelLaunch {
                file: file.to_path_buf(),
                argument: argument.map(Path::to_path_buf),
            });
        Ok(())
    }
}

#[cfg(windows)]
mod sys {
    use crate::core::error::{CoreError, Result};
    use std::path::Path;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// `ERROR_CANCELLED`: the user said No at the UAC prompt.
    const WIN32_CANCELLED: u32 = 1223;

    fn win32_code(error: &windows::core::Error) -> u32 {
        let hr = error.code().0.cast_unsigned();
        if hr & 0xFFFF_0000 == 0x8007_0000 {
            hr & 0xFFFF
        } else {
            hr
        }
    }

    pub(super) fn open(file: &Path, argument: Option<&Path>) -> Result<()> {
        let file_w = HSTRING::from(file.as_os_str());
        // The argument is always a System32 path built by the module; quoting
        // it is the whole escaping job (no `"` can appear in a Windows path).
        let parameters = argument.map(|path| HSTRING::from(format!("\"{}\"", path.display())));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or(u32::MAX),
            fMask: SEE_MASK_NOASYNC,
            lpFile: PCWSTR(file_w.as_ptr()),
            lpParameters: parameters
                .as_ref()
                .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: every pointer in `info` outlives the call. The shell, not
        // `CreateProcess`, so the window's own elevation request is honoured.
        match unsafe { ShellExecuteExW(&mut info) } {
            Ok(()) => Ok(()),
            Err(err) if win32_code(&err) == WIN32_CANCELLED => Ok(()),
            Err(err) => Err(CoreError::LegacyPanel(format!(
                "could not open `{}`: {err}",
                file.display()
            ))),
        }
    }
}

#[cfg(not(windows))]
mod sys {
    use crate::core::error::{CoreError, Result};
    use std::path::Path;

    pub(super) fn open(file: &Path, _argument: Option<&Path>) -> Result<()> {
        Err(CoreError::LegacyPanel(format!(
            "there are no Windows panels on this platform (asked for `{}`)",
            file.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_process_cannot_open_a_real_panel() {
        let err = SystemPanelLauncher
            .open(Path::new(r"C:\Windows\System32\control.exe"), None)
            .expect_err("a test build must not reach the shell");
        assert!(matches!(err, CoreError::LegacyPanel(_)));
    }

    #[test]
    fn the_fake_records_instead_of_launching() {
        let fake = FakePanelLauncher::with_missing([std::path::PathBuf::from(r"C:\gone.exe")]);
        assert!(!fake.exists(Path::new(r"C:\gone.exe")));
        assert!(fake.exists(Path::new(r"C:\here.exe")));
        fake.open(Path::new(r"C:\here.exe"), Some(Path::new(r"C:\x.cpl")))
            .expect("fake open");
        assert_eq!(
            fake.launches(),
            vec![PanelLaunch {
                file: r"C:\here.exe".into(),
                argument: Some(r"C:\x.cpl".into()),
            }]
        );
    }
}
