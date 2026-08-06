//! Shutdown / restart / sleep, behind a trait so a test can never trigger one.
//!
//! # Safety rule
//!
//! A unit test that actually shut the developer's machine down would be a
//! catastrophic bug, so this file makes it impossible in three independent ways:
//!
//! 1. The timer engine only ever holds an `Arc<dyn PowerController>`; it has no
//!    way to name a concrete implementation.
//! 2. [`SystemPower`] — the only implementation that touches the OS — refuses
//!    to act when `cfg!(test)` is true, which covers every unit test in this
//!    crate. There is a test below proving that refusal.
//! 3. The application only installs [`SystemPower`] in `lib.rs`, and only when
//!    the `--dry-run` flag is absent. Tests never run `lib.rs::run()`.
//!
//! [`DryRunPower`] records what *would* have happened into a log the tests and
//! the `--dry-run` build assert against. Nothing else.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, Result};

/// What the timer should do when it reaches zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PowerAction {
    /// Full shutdown (`shutdown /s /t 0`).
    Shutdown,
    /// Restart (`shutdown /r /t 0`).
    Restart,
    /// Suspend to RAM (`SetSuspendState`).
    Sleep,
}

impl PowerAction {
    /// Every variant, for exhaustive tests and UI lists.
    pub const ALL: [Self; 3] = [Self::Shutdown, Self::Restart, Self::Sleep];

    /// The `shutdown.exe` switch, where one applies.
    pub fn shutdown_exe_flag(self) -> Option<&'static str> {
        match self {
            Self::Shutdown => Some("/s"),
            Self::Restart => Some("/r"),
            // Sleep is not a `shutdown.exe` mode; it goes through the API.
            Self::Sleep => None,
        }
    }

    /// Stable slug used in logs and settings.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Shutdown => "shutdown",
            Self::Restart => "restart",
            Self::Sleep => "sleep",
        }
    }
}

/// One line of the dry-run log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerEvent {
    /// What would have been done.
    pub action: PowerAction,
    /// When the timer fired.
    pub at: DateTime<Utc>,
}

/// Performs a power action. The only thing the shutdown timer is allowed to
/// hold.
pub trait PowerController: Send + Sync {
    /// Apply `action` now.
    ///
    /// # Errors
    /// Returns [`CoreError::PowerAction`] when the OS refuses, and
    /// [`CoreError::PowerActionBlocked`] when the call was made from a context
    /// that must never reach the OS.
    fn apply(&self, action: PowerAction) -> Result<()>;

    /// Whether this controller only pretends. The UI shows a banner when true
    /// so nobody mistakes a dry run for the real thing.
    fn is_dry_run(&self) -> bool {
        false
    }
}

/// Records power actions instead of performing them.
///
/// This is what `--dry-run` installs and the only controller any test sees.
#[derive(Debug, Clone, Default)]
pub struct DryRunPower {
    log: Arc<Mutex<Vec<PowerEvent>>>,
}

impl DryRunPower {
    /// A controller with an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything that would have been executed, oldest first.
    pub fn log(&self) -> Vec<PowerEvent> {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether the log is empty. Reads better than `log().is_empty()` in tests.
    pub fn is_empty(&self) -> bool {
        self.log().is_empty()
    }
}

impl PowerController for DryRunPower {
    fn apply(&self, action: PowerAction) -> Result<()> {
        let event = PowerEvent {
            action,
            at: Utc::now(),
        };
        tracing::warn!(
            action = action.slug(),
            "dry run: power action logged, not executed"
        );
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
        Ok(())
    }

    fn is_dry_run(&self) -> bool {
        true
    }
}

/// The real thing. Constructed in exactly one place: `lib.rs::run()`, and only
/// when `--dry-run` was not passed.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPower;

#[cfg(windows)]
impl PowerController for SystemPower {
    fn apply(&self, action: PowerAction) -> Result<()> {
        // Guard 2 of 3 (see the module header). `cfg!(test)` is true for every
        // unit test compiled into this crate, so no test can get past here even
        // if one managed to construct a `SystemPower`.
        if cfg!(test) {
            return Err(CoreError::PowerActionBlocked(action.slug()));
        }

        match action {
            PowerAction::Shutdown | PowerAction::Restart => {
                let flag = action
                    .shutdown_exe_flag()
                    .ok_or(CoreError::PowerActionBlocked(action.slug()))?;
                tracing::warn!(action = action.slug(), "executing power action");
                std::process::Command::new("shutdown.exe")
                    .args([flag, "/t", "0"])
                    .spawn()
                    .map_err(|err| {
                        CoreError::PowerAction(format!("could not start shutdown.exe: {err}"))
                    })?;
                Ok(())
            }
            PowerAction::Sleep => {
                tracing::warn!(action = action.slug(), "executing power action");
                // SAFETY: `SetSuspendState` takes three BOOLEANs and has no
                // pointer arguments, so there is nothing to get wrong beyond
                // the values themselves: hibernate = false (sleep, not
                // hibernate), force = false (let apps veto), wake_events =
                // false (do not arm wake timers).
                let ok =
                    unsafe { windows::Win32::System::Power::SetSuspendState(false, false, false) };
                if ok {
                    Ok(())
                } else {
                    Err(CoreError::PowerAction(
                        "SetSuspendState refused to suspend the system".to_owned(),
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_a_slug_and_the_right_shutdown_flag() {
        assert_eq!(PowerAction::Shutdown.shutdown_exe_flag(), Some("/s"));
        assert_eq!(PowerAction::Restart.shutdown_exe_flag(), Some("/r"));
        assert_eq!(PowerAction::Sleep.shutdown_exe_flag(), None);
        for action in PowerAction::ALL {
            assert!(!action.slug().is_empty());
        }
    }

    #[test]
    fn power_action_wire_format_matches_typescript() {
        let json = serde_json::to_string(&PowerAction::ALL).expect("serialize");
        assert_eq!(json, r#"["shutdown","restart","sleep"]"#);
    }

    #[test]
    fn the_dry_run_controller_records_instead_of_acting() {
        let power = DryRunPower::new();
        assert!(power.is_empty());
        assert!(power.is_dry_run());

        power.apply(PowerAction::Shutdown).expect("dry run");
        power.apply(PowerAction::Sleep).expect("dry run");

        let log: Vec<_> = power.log().into_iter().map(|e| e.action).collect();
        assert_eq!(log, vec![PowerAction::Shutdown, PowerAction::Sleep]);
    }

    #[test]
    fn the_dry_run_log_is_shared_between_clones() {
        let power = DryRunPower::new();
        let clone = power.clone();
        clone.apply(PowerAction::Restart).expect("dry run");
        assert_eq!(power.log().len(), 1);
    }

    /// Guard 2 of 3: even a direct call to the real controller from a test is
    /// refused. If this test ever fails, a test run can shut the machine down.
    #[cfg(windows)]
    #[test]
    fn the_system_controller_refuses_to_act_inside_tests() {
        for action in PowerAction::ALL {
            let err = SystemPower
                .apply(action)
                .expect_err("a test must never reach the OS power API");
            assert!(
                matches!(err, CoreError::PowerActionBlocked(_)),
                "expected the test guard to fire, got {err}"
            );
        }
    }
}
