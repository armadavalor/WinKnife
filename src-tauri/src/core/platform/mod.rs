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

pub mod env;
pub mod lock;
pub mod power;
pub mod recycle_bin;

pub use env::{EnvResolver, MapEnv, SystemEnv};
pub use lock::{AlwaysUnlocked, LockProbe, LockState, SystemLockProbe};
pub use power::{DryRunPower, PowerAction, PowerController, PowerEvent};
pub use recycle_bin::{RecycleBin, RecycleBinStats, StubRecycleBin};

#[cfg(windows)]
pub use power::SystemPower;
#[cfg(windows)]
pub use recycle_bin::WindowsRecycleBin;
