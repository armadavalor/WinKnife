//! One-at-a-time lock for the operations that move files into or out of the
//! quarantine (issue #39).
//!
//! Clean-up, restore and purge all run under the database mutex, so a double
//! click never ran two at once; it queued the second run to start the moment
//! the first finished. This lock is taken *before* the database, so the second
//! call is refused with [`CoreError::Busy`] instead.
//!
//! The lock lives on [`crate::core::quarantine::Quarantine`] and is shared by
//! its clones. It does not replace any of the executor's three gates (K-006);
//! it only decides whether a run may start.

use std::sync::{Mutex, PoisonError};

use crate::core::error::CoreError;

/// What holds the lock, for the "already running" message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Executing approved recommendations.
    Clean,
    /// Restoring a batch or an item from the quarantine.
    Restore,
    /// Permanently removing expired or chosen quarantine items.
    Purge,
}

impl Operation {
    /// The subject of the "already running" sentence.
    pub fn label(self) -> &'static str {
        match self {
            Self::Clean => "A clean-up",
            Self::Restore => "A restore",
            Self::Purge => "A purge",
        }
    }
}

/// Holds which operation, if any, is running.
#[derive(Debug, Default)]
pub struct OperationLock {
    running: Mutex<Option<Operation>>,
}

impl OperationLock {
    /// Start `operation`, or refuse because another one is running. The lock
    /// is released when the guard drops, including on an error or a panic.
    pub fn begin(&self, operation: Operation) -> Result<OperationGuard<'_>, CoreError> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(current) = *running {
            return Err(CoreError::Busy(current));
        }
        *running = Some(operation);
        Ok(OperationGuard { lock: self })
    }
}

/// Proof that an operation is running; dropping it ends the operation.
#[derive(Debug)]
#[must_use = "the operation ends as soon as the guard is dropped"]
pub struct OperationGuard<'a> {
    lock: &'a OperationLock,
}

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        *self
            .lock
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_operation_is_refused_with_the_running_one_named() {
        let lock = OperationLock::default();
        let _clean = lock.begin(Operation::Clean).expect("free");
        let err = lock.begin(Operation::Purge).expect_err("busy");
        assert!(matches!(err, CoreError::Busy(Operation::Clean)));
        assert_eq!(
            err.to_string(),
            "A clean-up is already running; wait for it to finish"
        );
    }

    #[test]
    fn dropping_the_guard_releases_the_lock() {
        let lock = OperationLock::default();
        drop(lock.begin(Operation::Restore).expect("free"));
        let _guard = lock
            .begin(Operation::Purge)
            .expect("free again after the drop");
    }

    #[test]
    fn a_panic_while_holding_the_guard_releases_the_lock() {
        let lock = OperationLock::default();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lock.begin(Operation::Clean).expect("free");
            panic!("probe");
        }));
        assert!(caught.is_err());
        let _guard = lock
            .begin(Operation::Clean)
            .expect("the unwind dropped the guard");
    }
}
