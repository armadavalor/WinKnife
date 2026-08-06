//! Helpers shared by this crate's unit tests. Compiled only under `cfg(test)`.
//!
//! # Why a skip has to be loud
//!
//! Three safety tests — "the walker never follows a link", "a link into a
//! protected directory is refused", "a walk root that resolves into a protected
//! location is refused" — need a real reparse point, and creating one on
//! Windows needs Developer Mode or elevation. They used to `return` quietly
//! when that failed, so on an unprivileged machine all three passed while
//! asserting nothing, and the suite was green (security review M11).
//!
//! A green suite hiding skipped safety tests is the exact failure mode this
//! project cares about, so the rule is now: **the privileged operation is
//! attempted, and if it is unavailable the test fails** — unless the operator
//! explicitly opted out for this machine by setting
//! [`ALLOW_SKIP_LINK_TESTS`]. CI does not set it (`.github/workflows/ci.yml`),
//! and `windows-latest` can create symlinks, so CI really does exercise them.

/// Opt-out for a developer machine that cannot create links.
///
/// Set to `1` to turn the three link-dependent safety tests into visible skips
/// instead of failures. CI must never set it.
pub(crate) const ALLOW_SKIP_LINK_TESTS: &str = "WINKNIFE_ALLOW_SKIP_LINK_TESTS";

/// Whether the operator opted this machine out of the link-dependent tests.
fn skipping_is_allowed() -> bool {
    std::env::var_os(ALLOW_SKIP_LINK_TESTS).is_some_and(|value| value == "1")
}

/// Report a skip somewhere `cargo test` cannot swallow it.
///
/// `println!`/`eprintln!` inside a *passing* test go to libtest's per-test
/// capture buffer, which is only printed for failures or under `--nocapture` —
/// so the obvious way to announce a skip is also the way to hide it. Writing to
/// the process's own stderr handle bypasses that capture, which is the point:
/// a skipped safety test has to be visible in the same output a green run
/// produces.
fn announce(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{message}");
}

/// The 8.3 short alias of `path`, or `None` when the call fails.
///
/// Two spellings of one directory is the shape several safety tests need:
/// `C:\Users\RUNNER~1\...` and `C:\Users\runneradmin\...` open the same folder,
/// and the protected-path list has to recognise both. Returns the long name
/// unchanged when 8.3 generation is off for the volume, which the callers check
/// for rather than asserting on a fixture that is not there.
#[cfg(windows)]
pub(crate) fn short_path_name(path: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated and outlives the call, and the buffer
    // length the API is given is its real length in `u16`s.
    let written = unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buffer)) } as usize;
    if written == 0 || written >= buffer.len() {
        return None;
    }
    Some(std::path::PathBuf::from(OsString::from_wide(
        &buffer[..written],
    )))
}

/// Called when a privileged operation a safety test depends on is unavailable.
///
/// Panics — the test fails — unless [`ALLOW_SKIP_LINK_TESTS`] is set to `1`, in
/// which case it prints a skip line that is visible in `cargo test` output and
/// returns so the caller can bail out.
///
/// # Panics
/// When the privileged operation is unavailable and no opt-out is set.
#[track_caller]
pub(crate) fn privileged_operation_unavailable(test: &str, detail: &str) {
    if skipping_is_allowed() {
        announce(&format!(
            "!! SKIPPED SAFETY TEST {test}: {detail} (opted out with \
             {ALLOW_SKIP_LINK_TESTS}=1). It asserted nothing on this run."
        ));
        return;
    }
    panic!(
        "{test} could not run: {detail}.\n\
         This is a safety test, and a silent skip is what the last security review \
         rejected — so it fails instead.\n\
         Fix: enable Windows Developer Mode (Settings > System > For developers) or run \
         the suite elevated, so links can be created.\n\
         Opt out on this machine only with {ALLOW_SKIP_LINK_TESTS}=1. CI must never set it."
    );
}
