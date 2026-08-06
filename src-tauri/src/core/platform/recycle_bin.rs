//! The recycle bin, through the shell API rather than a raw `$Recycle.Bin` walk.
//!
//! Why the shell API: the on-disk layout of `$Recycle.Bin` is undocumented and
//! per-SID, and walking it would let us "restore" files in a way Explorer does
//! not understand. `SHQueryRecycleBin` reports the real size and count;
//! `SHEmptyRecycleBin` is the one action in this whole app that does not go
//! through quarantine, because the recycle bin *is* Windows' quarantine.
//!
//! # Safety rule
//!
//! Emptying is irreversible, so `WindowsRecycleBin::empty` carries the same
//! process-global guard as the power controller: it refuses until `lib.rs::run()`
//! has armed real OS effects. It used to check `cfg!(test)`, which is false in
//! an integration-test binary and so guarded nothing there (security review H5).
//! The type is crate-private and not re-exported; everything above this file
//! only ever holds a `dyn RecycleBin`, and tests get [`StubRecycleBin`].

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, Result};

/// What the recycle bin currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleBinStats {
    /// Number of items across all drives.
    pub item_count: u64,
    /// Total size in bytes.
    pub size_bytes: u64,
}

impl RecycleBinStats {
    /// Whether there is anything to reclaim.
    pub fn is_empty(self) -> bool {
        self.item_count == 0 || self.size_bytes == 0
    }
}

/// Reads and empties the recycle bin.
pub trait RecycleBin: Send + Sync {
    /// Current contents. Read-only, safe to call from `scan`.
    ///
    /// # Errors
    /// Returns [`CoreError::RecycleBin`] when the shell API refuses.
    fn query(&self) -> Result<RecycleBinStats>;

    /// Empty the bin. Irreversible — only ever called from `execute` after an
    /// explicit confirmation that names this action separately.
    ///
    /// # Errors
    /// Returns [`CoreError::RecycleBin`] when the shell API refuses, or
    /// [`CoreError::PowerActionBlocked`]-style refusal in test builds.
    fn empty(&self) -> Result<()>;
}

/// The real shell-API implementation. Reachable only through
/// `platform::real_recycle_bin`.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WindowsRecycleBin;

#[cfg(windows)]
impl RecycleBin for WindowsRecycleBin {
    fn query(&self) -> Result<RecycleBinStats> {
        use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};

        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            i64Size: 0,
            i64NumItems: 0,
        };

        // SAFETY: `info` is a correctly sized, fully initialised SHQUERYRBINFO
        // owned by this stack frame; passing `None` for the root queries every
        // drive, which is what the documentation prescribes.
        let hr = unsafe { SHQueryRecycleBinW(None, &mut info) };
        hr.map_err(|err| CoreError::RecycleBin(format!("SHQueryRecycleBin failed: {err}")))?;

        Ok(RecycleBinStats {
            item_count: info.i64NumItems.max(0) as u64,
            size_bytes: info.i64Size.max(0) as u64,
        })
    }

    fn empty(&self) -> Result<()> {
        use windows::Win32::UI::Shell::{
            SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND,
        };

        // The same guard as the power controller: no test binary of any kind
        // may reach this call, because only the composition root can arm it.
        if !crate::core::platform::real_os_effects_armed() {
            return Err(CoreError::RecycleBinBlocked);
        }

        tracing::warn!("emptying the recycle bin (not reversible, user-confirmed)");
        // SAFETY: no window handle and no root path means "every drive, no UI",
        // which matches the flags. There are no pointers to keep alive.
        let hr = unsafe {
            SHEmptyRecycleBinW(
                None,
                None,
                SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
            )
        };
        hr.map_err(|err| CoreError::RecycleBin(format!("SHEmptyRecycleBin failed: {err}")))
    }
}

/// A recycle bin that reports contents and records `empty()` calls without
/// touching anything. Used by every test and on non-Windows builds.
///
/// Emptying it really does empty it, because the executor sizes what it
/// reclaimed by asking again afterwards: a stub that kept reporting a full bin
/// after being emptied would make that measurement untestable.
#[derive(Debug, Clone, Default)]
pub struct StubRecycleBin {
    stats: Arc<Mutex<RecycleBinStats>>,
    emptied: Arc<Mutex<u32>>,
}

impl StubRecycleBin {
    /// A bin holding `item_count` items totalling `size_bytes`.
    pub fn new(item_count: u64, size_bytes: u64) -> Self {
        Self {
            stats: Arc::new(Mutex::new(RecycleBinStats {
                item_count,
                size_bytes,
            })),
            emptied: Arc::new(Mutex::new(0)),
        }
    }

    /// How many times `empty()` was called.
    pub fn empty_calls(&self) -> u32 {
        *self
            .emptied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl RecycleBin for StubRecycleBin {
    fn query(&self) -> Result<RecycleBinStats> {
        Ok(*self
            .stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    fn empty(&self) -> Result<()> {
        *self
            .emptied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        *self
            .stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = RecycleBinStats::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_report_emptiness() {
        assert!(RecycleBinStats::default().is_empty());
        assert!(!RecycleBinStats {
            item_count: 3,
            size_bytes: 100,
        }
        .is_empty());
    }

    #[test]
    fn stats_wire_format_matches_typescript() {
        let json = serde_json::to_string(&RecycleBinStats {
            item_count: 3,
            size_bytes: 100,
        })
        .expect("serialize");
        assert_eq!(json, r#"{"itemCount":3,"sizeBytes":100}"#);
    }

    #[test]
    fn the_stub_reports_what_it_was_built_with_and_counts_empties() {
        let bin = StubRecycleBin::new(4, 2_048);
        assert_eq!(
            bin.query().expect("query"),
            RecycleBinStats {
                item_count: 4,
                size_bytes: 2_048,
            }
        );
        assert_eq!(bin.empty_calls(), 0);
        bin.empty().expect("stub empty");
        assert_eq!(bin.empty_calls(), 1);
    }

    /// The real `SHEmptyRecycleBin` must be unreachable from a test run.
    #[cfg(windows)]
    #[test]
    fn the_real_recycle_bin_refuses_to_empty_inside_tests() {
        let err = WindowsRecycleBin
            .empty()
            .expect_err("a test must never empty the real recycle bin");
        assert!(
            matches!(err, CoreError::RecycleBinBlocked),
            "expected the test guard to fire, got {err}"
        );
    }
}
