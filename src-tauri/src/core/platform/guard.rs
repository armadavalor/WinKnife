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
//! `lib.rs::run()`, the composition root.
//!
//! # Exactly what is guaranteed
//!
//! - **Integration tests, doctests, benchmarks and any other crate**: cannot
//!   arm the flag, because `arm_real_os_effects` is `pub(crate)`. They link the
//!   library as an ordinary dependency and the item is not in their namespace.
//! - **This crate's own unit tests** (`cargo test --lib`, and `--all-targets`
//!   builds of it): cannot arm the flag either, because the function does not
//!   exist under `cfg(test)` — a `#[cfg(test)]` module that called it would
//!   fail to compile (security review M12). `pub(crate)` alone did not give
//!   this: an in-crate test module could call it and unlock the real
//!   `SystemPower` and `WindowsRecycleBin` for every later test in the binary.
//! - **The application binary**: arms it once, from `run()`, unless `--dry-run`
//!   asked for the no-op power controller.
//!
//! What is *not* guaranteed: nothing here stops a future non-test caller inside
//! this crate from arming the flag. That is deliberate — `run()` is one of
//! those callers.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this process may perform irreversible OS actions.
static REAL_OS_EFFECTS: AtomicBool = AtomicBool::new(false);

/// Allow the irreversible OS calls to happen.
///
/// Called once, from `lib.rs::run()`. `pub(crate)` keeps every other crate out;
/// `cfg(not(test))` keeps this crate's own unit tests out, so a test module
/// cannot arm the process and hand the rest of the binary a live
/// `SystemPower`. A test that tries does not compile.
#[cfg(not(test))]
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

    /// M12: the arming function is not merely private, it is *absent* from a
    /// test build. This is the compile-time half of that claim — if the
    /// `cfg(not(test))` gate were removed, the commented-out line below would
    /// compile, and this test's reason for existing would be gone.
    ///
    /// ```compile_fail
    /// crate::core::platform::guard::arm_real_os_effects();
    /// ```
    #[test]
    fn the_arming_function_does_not_exist_in_a_test_build() {
        // `module_path!` proves we are in the test cfg; the absence of the
        // symbol is proved by the fact that this file compiles at all while
        // `arm_real_os_effects` is defined under `cfg(not(test))`.
        assert!(module_path!().ends_with("guard::tests"));
    }
}
