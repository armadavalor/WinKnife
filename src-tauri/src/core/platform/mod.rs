//! Every OS-specific call the app makes, behind a small mockable trait.
//!
//! Why: three of the operations in this app are irreversible on a developer's
//! real machine — shutting it down, emptying the recycle bin, suspending it.
//! Routing them all through traits means a test can only ever reach a fake, and
//! the real implementations live in exactly one place where they can be audited
//! (CONVENTIONS.md: "Windows-specific calls via the `windows` crate behind small
//! wrapper fns (mockable in tests)").
//!
//! The same idea covers the environment: rule packs expand `%TEMP%`, and a test
//! that expanded it for real would scan the developer's actual temp folder. The
//! [`EnvResolver`] indirection points fixtures at a tempdir instead.

//! The two implementations that can do something irreversible — `SystemPower`
//! and `WindowsRecycleBin` — are deliberately not re-exported and their modules
//! are crate-private, so no test can name them. They are reachable only as the
//! trait objects [`real_power_controller`] and [`real_recycle_bin`] hand back,
//! and those refuse to act until [`guard::arm_real_os_effects`] has run, which
//! only `lib.rs::run()` can do (security review H5).

pub(crate) mod env;
pub(crate) mod execution_state;
pub(crate) mod explorer;
pub(crate) mod file_id;
pub(crate) mod guard;
pub(crate) mod lock;
pub(crate) mod message;
pub(crate) mod pinned;
pub(crate) mod power;
pub(crate) mod process;
pub(crate) mod recycle_bin;
pub(crate) mod registry;
pub(crate) mod restore_point;
pub(crate) mod signature;
pub(crate) mod tasks;
pub(crate) mod volume;

use std::sync::Arc;

pub use env::{EnvResolver, MapEnv, SystemEnv};
pub use execution_state::{AwakeRequest, ExecutionState, RecordedCall, RecordingExecutionState};
pub use explorer::reveal_in_explorer;
pub use file_id::{file_identity, try_file_identity, FileId};
pub use guard::real_os_effects_armed;
pub use lock::{AlwaysUnlocked, LockProbe, LockState, SystemLockProbe};
pub use message::show_fatal_error;
pub use power::{DryRunPower, PowerAction, PowerController, PowerEvent};
pub use process::{CachedProcessProbe, ProcessProbe, StubProcessProbe, SystemProcessProbe};
pub use recycle_bin::{RecycleBin, RecycleBinStats, StubRecycleBin};
pub use registry::{
    Hive, MapRegistry, RegValue, Registry, SystemRegistry, REG_TYPE_BINARY, REG_TYPE_EXPAND_SZ,
    REG_TYPE_SZ,
};
pub use restore_point::{
    create_in_helper, failed_exit_code, failure_from_exit_code, FakeRestorePointPlatform, HelperRun,
    RestorePointPlatform, SystemRestorePoints, CREATE_RESTORE_POINT_FLAG, EXIT_CREATED,
    EXIT_MAYBE_SKIPPED, EXIT_PROTECTION_OFF, EXIT_SKIPPED, WIN32_CANCELLED,
};
pub use signature::{MapSignatures, Signature, SignatureProbe, SystemSignatureProbe};
pub use tasks::{LogonTask, MapTaskScheduler, SystemTaskScheduler, TaskScheduler};
pub use volume::{DriveInfo, StubVolumes, SystemVolumes, Volumes};

// `cfg(not(test))` mirrors the definition: the arming function does not exist
// in a test build, so neither does its re-export (security review M12).
#[cfg(not(test))]
pub(crate) use guard::arm_real_os_effects;

/// The power controller that talks to the operating system.
///
/// Public so a test can prove it refuses; useless to a test for the same
/// reason it is safe — nothing outside the composition root can arm it.
pub fn real_power_controller() -> Arc<dyn PowerController> {
    #[cfg(windows)]
    {
        Arc::new(power::SystemPower)
    }
    #[cfg(not(windows))]
    {
        // No power API to reach off Windows; log instead of pretending.
        Arc::new(DryRunPower::new())
    }
}

/// The execution-state API that talks to the OS (keep-awake).
pub fn real_execution_state() -> Arc<dyn ExecutionState> {
    #[cfg(windows)]
    {
        Arc::new(execution_state::SystemExecutionState)
    }
    #[cfg(not(windows))]
    {
        // No such API off Windows; record instead of pretending.
        Arc::new(RecordingExecutionState::new())
    }
}

/// The recycle bin that talks to the shell API.
pub fn real_recycle_bin() -> Arc<dyn RecycleBin> {
    #[cfg(windows)]
    {
        Arc::new(recycle_bin::WindowsRecycleBin)
    }
    #[cfg(not(windows))]
    {
        // Nothing to query off Windows; report an empty bin rather than lying.
        Arc::new(StubRecycleBin::default())
    }
}
