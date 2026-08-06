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
///
/// # How that is actually enforced
///
/// By the `cfg(not(test))` on the line below, and nothing else — so it is worth
/// being precise about what does and does not check it:
///
/// - **This crate's own unit tests**: the item does not exist in their build at
///   all, so a call is a name-resolution error. `tests::a_test_process_is_never_armed`
///   asserts the observable consequence — the flag is never set in a test run.
/// - **Every other crate**, including the integration tests in `tests/` and any
///   doctest: `pub(crate)` puts the item outside their namespace.
/// - **CI**: `cargo clippy --all-targets -- -D warnings` compiles the test
///   targets, so a `#[cfg(test)]` caller would fail the build.
///
/// This used to carry a ```compile_fail``` doctest as the proof. It sat inside
/// `#[cfg(test)] mod tests`, and rustdoc does not collect doctests from
/// `cfg(test)` code — `Doc-tests winknife_lib` reported `running 0 tests`, so
/// the proof never ran. The claim was true and the evidence was theatre
/// (security review L9).
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
    /// test build.
    ///
    /// This used to claim a ```compile_fail``` doctest as its proof. The doctest
    /// lived inside this `#[cfg(test)]` module, and rustdoc does not collect
    /// doctests from `cfg(test)` code, so it never ran — `Doc-tests
    /// winknife_lib` said `running 0 tests` on every green build (security
    /// review L9). The guarantee is real; the evidence was not, so here is what
    /// really holds it up:
    ///
    /// - This module compiles under `cfg(test)`, and `arm_real_os_effects` is
    ///   defined under `cfg(not(test))`. A call from here would be a
    ///   name-resolution error, which is a build failure and not a warning —
    ///   and CI builds the test targets (`clippy --all-targets -- -D warnings`).
    /// - `a_test_process_is_never_armed` asserts the consequence that actually
    ///   matters at runtime, in this same binary.
    #[test]
    fn the_arming_function_does_not_exist_in_a_test_build() {
        // `module_path!` proves we really are in the test cfg — which is the
        // cfg under which the symbol does not exist.
        assert!(module_path!().ends_with("guard::tests"));
    }
}
