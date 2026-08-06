//! Showing a path in File Explorer.
//!
//! Module spec 04 calls the cross-links "the Swiss-knife glue" and says not to
//! skip them: a treemap that shows you a 12 GB folder and then cannot open it is
//! half a tool.
//!
//! This is the only place in the app that starts another process, so it lives
//! here with the rest of the OS surface rather than inside a module. It changes
//! nothing — `explorer /select` opens a window — but it is still gated on
//! [`real_os_effects_armed`](super::real_os_effects_armed), because a test suite
//! that opened a few hundred Explorer windows would be its own kind of damage.

use std::path::Path;

use crate::core::error::{CoreError, Result};

/// Open File Explorer at `path`, selecting it when it is a file.
///
/// # Errors
/// Returns an error when the path does not exist, when Explorer could not be
/// started, or when this build is not allowed to reach the OS.
pub fn reveal_in_explorer(path: &Path) -> Result<()> {
    if !super::real_os_effects_armed() {
        return Err(CoreError::Explorer(
            "this build must never open an Explorer window".to_owned(),
        ));
    }
    if !path.exists() {
        return Err(CoreError::Explorer(format!(
            "`{}` is not there any more",
            path.display()
        )));
    }
    spawn(path)
}

#[cfg(windows)]
fn spawn(path: &Path) -> Result<()> {
    use std::process::{Command, Stdio};

    // `/select,` needs the path glued to it with no space, and Explorer wants
    // backslashes. The path is passed as one argument, so nothing here is
    // shell-interpreted.
    let mut argument = std::ffi::OsString::from("/select,");
    argument.push(path.as_os_str());

    Command::new("explorer.exe")
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| CoreError::Explorer(format!("could not start Explorer: {err}")))?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn(path: &Path) -> Result<()> {
    Err(CoreError::Explorer(format!(
        "there is no File Explorer on this platform (asked for `{}`)",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test run must not be able to fill the developer's screen with windows.
    #[test]
    fn a_test_process_cannot_open_explorer() {
        assert!(!super::super::real_os_effects_armed());
        let err = reveal_in_explorer(Path::new("C:\\"))
            .expect_err("a test build must not reach Explorer");
        assert!(matches!(err, CoreError::Explorer(_)));
    }
}
