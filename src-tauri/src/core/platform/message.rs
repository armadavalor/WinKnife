//! A native error box for a start that cannot go on.
//!
//! The release build has no console (`windows_subsystem = "windows"`), so an
//! error out of Tauri's `setup` used to end the process with nothing on
//! screen. The one case worth a box is a database written by a newer build
//! (round 3): the user has to be told to install that version again rather
//! than see the app vanish.
//!
//! A modal box blocks until it is dismissed, so it is gated on
//! [`real_os_effects_armed`](super::real_os_effects_armed) like everything else
//! here that reaches the desktop: a test can never open one.

/// Show `text` in a modal error box titled `title`. Returns whether a box
/// was shown; a build that may not reach the OS shows nothing.
pub fn show_fatal_error(title: &str, text: &str) -> bool {
    if !super::real_os_effects_armed() {
        return false;
    }
    show(title, text)
}

#[cfg(windows)]
fn show(title: &str, text: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
    };

    // SAFETY: both strings are owned, NUL-terminated wide strings that outlive
    // the call, and no owner window is passed.
    let pressed = unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        )
    };
    pressed.0 != 0
}

#[cfg(not(windows))]
fn show(_title: &str, _text: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test build is never armed, so it can never block on a modal box.
    #[test]
    fn a_test_build_never_shows_a_box() {
        assert!(!show_fatal_error("WinKnife", "a test"));
    }
}
