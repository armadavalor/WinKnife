//! The one switch that lets this process touch the operating system for real.
//!
//! # Why a runtime flag and not `cfg!(test)`
//!
//! Two operations in this app cannot be undone: shutting the machine down and
//! emptying the recycle bin. Both used to be guarded by `if cfg!(test)`, which
//! reads well and does not work: `cfg(test)` is set for the *compilation unit
//! being tested*, so it is true inside this crate's own unit tests and false in
//! every integration test in `tests/`, which links the library as an ordinary
//! dependency. Anything in `tests/` that reached those types would have run the
//! real call against the developer's machine (security review H5).
//!
//! A process-global flag has no such seam. It is armed in exactly one place —
//! `lib.rs::run()`, the composition root — and `arm_real_os_effects` is
//! `pub(crate)`, so nothing outside this crate can arm it. Unit tests,
//! integration tests, doctests, benchmarks and any future binary that forgets
//! to arm it all get the same refusal, for the same reason.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this process may perform irreversible OS actions.
static REAL_OS_EFFECTS: AtomicBool = AtomicBool::new(false);

/// Allow the irreversible OS calls to happen.
///
/// Called once, from `lib.rs::run()`. Deliberately `pub(crate)`: a test can
/// reach the real implementations through the composition-root helpers, and the
/// only reason that is safe is that no test can turn this on.
pub(crate) fn arm_real_os_effects() {
    if REAL_OS_EFFECTS.swap(true, Ordering::SeqCst) {
        tracing::warn!("real OS effects were armed more than once");
    } else {
        tracing::info!("real OS effects armed by the composition root");
    }
}

/// Whether irreversible OS actions are allowed in this process.
///
/// False in every test binary, which is the point.
pub fn real_os_effects_armed() -> bool {
    REAL_OS_EFFECTS.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard that matters: a test run has not armed anything, and cannot.
    #[test]
    fn a_test_process_is_never_armed() {
        assert!(
            !real_os_effects_armed(),
            "a test run must never be allowed to reach an irreversible OS call"
        );
    }
}
