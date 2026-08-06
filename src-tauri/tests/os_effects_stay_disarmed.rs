//! The guard that stops a test run shutting the developer's machine down or
//! emptying their recycle bin, proved from the side of the boundary where the
//! old guard did not exist.
//!
//! `SystemPower` and `WindowsRecycleBin` used to refuse when `cfg!(test)` was
//! true. `cfg(test)` is set for the compilation unit *being tested*, so it is
//! true inside the library's own unit tests and **false here** — an integration
//! test links the library exactly as the application does. Anything in this
//! directory that reached those types would have run the real call
//! (security review H5).
//!
//! The replacement is a process-global flag that only `lib.rs::run()` can set.
//! This file holds the real implementations, calls them, and asserts they
//! refuse — which is only a safe thing to write because arming is `pub(crate)`
//! and no test can do it.

use cleanify_lib::core::error::CoreError;
use cleanify_lib::core::platform::{self, PowerAction};

/// The premise of everything below: no test process is ever armed.
#[test]
fn a_test_process_is_never_armed() {
    assert!(
        !platform::real_os_effects_armed(),
        "an integration-test binary must never be allowed to reach an irreversible OS call"
    );
}

/// Holding the real power controller and calling it must be safe, because it
/// refuses. If this test ever fails, `cargo test` can shut the machine down.
#[cfg(windows)]
#[test]
fn the_real_power_controller_refuses_from_an_integration_test() {
    let power = platform::real_power_controller();
    assert!(
        !power.is_dry_run(),
        "this must be the real controller, or the test proves nothing"
    );

    for action in PowerAction::ALL {
        let err = power
            .apply(action)
            .expect_err("a test must never reach the OS power API");
        assert!(
            matches!(err, CoreError::PowerActionBlocked(_)),
            "expected the process guard to fire for {action:?}, got {err}"
        );
    }
}

/// The same for the recycle bin, which is the other irreversible call.
#[cfg(windows)]
#[test]
fn the_real_recycle_bin_refuses_to_empty_from_an_integration_test() {
    let bin = platform::real_recycle_bin();

    let err = bin
        .empty()
        .expect_err("a test must never empty the real recycle bin");
    assert!(
        matches!(err, CoreError::RecycleBinBlocked),
        "expected the process guard to fire, got {err}"
    );

    // Reading is not destructive and stays allowed, because `scan` needs it.
    assert!(
        bin.query().is_ok(),
        "querying the bin is read-only and must keep working"
    );
}

/// Off Windows there is no real implementation to refuse; the helpers hand back
/// something that cannot touch anything either way.
#[cfg(not(windows))]
#[test]
fn there_is_nothing_real_to_reach_off_windows() {
    assert!(platform::real_power_controller().is_dry_run());
    assert!(platform::real_recycle_bin().empty().is_ok());
    let _ = PowerAction::ALL;
    let _: Option<CoreError> = None;
}
