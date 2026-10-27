//! Drive enumeration and capacity, behind a trait.
//!
//! The treemap's drive selector and its "total / free" bar need to know which
//! volumes exist and how big they are (module spec 04). That is three Win32
//! calls, and the same reasoning as everywhere else in this folder applies: a
//! test that asked the real machine would assert about the developer's disks.
//!
//! Everything here is **read-only**. There is no method that changes a volume,
//! so unlike the registry and the task scheduler this module needs no guard.

use std::path::{Path, PathBuf};

/// What kind of volume a drive letter is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DriveKind {
    /// An internal disk.
    Fixed,
    /// A USB stick, memory card or external disk.
    Removable,
    /// A mapped network drive.
    Network,
    /// An optical drive.
    Optical,
    /// A RAM disk or anything else Windows reports.
    Other,
}

/// One drive, as the treemap's selector shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    /// Root path, e.g. `C:\`.
    pub root: PathBuf,
    /// Volume label, when the volume has one.
    pub label: Option<String>,
    /// Total capacity in bytes; `0` when it could not be read.
    pub total_bytes: u64,
    /// Free bytes available to this user; `0` when it could not be read.
    pub free_bytes: u64,
    /// What kind of drive it is.
    pub kind: DriveKind,
}

impl DriveInfo {
    /// Bytes in use, never below zero.
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.free_bytes)
    }
}

/// Enumerate the machine's drives.
pub trait Volumes: Send + Sync {
    /// Every drive that is ready, in letter order.
    ///
    /// A drive that is present but not ready (an empty card reader) is left out
    /// rather than reported with zero capacity: a treemap of a drive that cannot
    /// be read is a lie about where space went.
    fn drives(&self) -> Vec<DriveInfo>;

    /// The drive `path` sits on, when it can be identified.
    fn for_path(&self, path: &Path) -> Option<DriveInfo> {
        // The prefix component of `D:\Games\x` is `D:`, and a drive root is
        // `D:\`. `Path::join` would *replace* rather than append here (the
        // argument starts with a separator), so the trailing separator is
        // appended as text and the comparison itself goes through
        // `path_compare` (standing rule).
        let prefix = path.components().next()?;
        let mut root = prefix.as_os_str().to_os_string();
        root.push("\\");
        let root = PathBuf::from(root);
        self.drives()
            .into_iter()
            .find(|drive| crate::core::path_compare::same_path(&drive.root, &root))
    }
}

/// A fake set of drives, for tests and for non-Windows builds.
#[derive(Debug, Default, Clone)]
pub struct StubVolumes {
    drives: Vec<DriveInfo>,
}

impl StubVolumes {
    /// No drives at all.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a fixed drive with the given capacity.
    #[must_use]
    pub fn with_drive(mut self, root: impl Into<PathBuf>, total: u64, free: u64) -> Self {
        self.drives.push(DriveInfo {
            root: root.into(),
            label: None,
            total_bytes: total,
            free_bytes: free,
            kind: DriveKind::Fixed,
        });
        self
    }
}

impl Volumes for StubVolumes {
    fn drives(&self) -> Vec<DriveInfo> {
        self.drives.clone()
    }
}

/// The drives of the machine this is running on.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemVolumes;

impl Volumes for SystemVolumes {
    fn drives(&self) -> Vec<DriveInfo> {
        #[cfg(windows)]
        {
            sys::drives()
        }
        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::{DriveInfo, DriveKind};
    use std::path::PathBuf;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };

    // `GetDriveTypeW`'s return codes. The `windows` crate keeps them in
    // `Win32::System::WindowsProgramming`, a feature this app pulls in for
    // nothing else; they are four documented, frozen integers.
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;

    pub(super) fn drives() -> Vec<DriveInfo> {
        // SAFETY: no arguments; returns a bitmask of present drive letters.
        let mask = unsafe { GetLogicalDrives() };
        let mut out = Vec::new();

        for index in 0..26u32 {
            if mask & (1 << index) == 0 {
                continue;
            }
            let letter = (b'A' + index as u8) as char;
            let root = format!("{letter}:\\");
            let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

            // SAFETY: `wide` is NUL-terminated and outlives the call.
            let kind = match unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } {
                t if t == DRIVE_FIXED => DriveKind::Fixed,
                t if t == DRIVE_REMOVABLE => DriveKind::Removable,
                t if t == DRIVE_REMOTE => DriveKind::Network,
                t if t == DRIVE_CDROM => DriveKind::Optical,
                _ => DriveKind::Other,
            };

            let mut free_to_caller = 0u64;
            let mut total = 0u64;
            // SAFETY: both out-parameters are live locals.
            let sized = unsafe {
                GetDiskFreeSpaceExW(
                    PCWSTR(wide.as_ptr()),
                    Some(&mut free_to_caller),
                    Some(&mut total),
                    None,
                )
            };
            if sized.is_err() || total == 0 {
                // Present but not ready — an empty card reader, or a network
                // drive whose server is asleep. Reporting it with zero capacity
                // would put an empty rectangle in a treemap that is supposed to
                // say where space went.
                continue;
            }

            let mut label_buffer = [0u16; 261];
            // SAFETY: the buffer length passed is its real length in `u16`s; the
            // filesystem-name out-parameters are not wanted.
            let labelled = unsafe {
                GetVolumeInformationW(
                    PCWSTR(wide.as_ptr()),
                    Some(&mut label_buffer),
                    None,
                    None,
                    None,
                    None,
                )
            };
            let label = labelled.is_ok().then(|| {
                String::from_utf16_lossy(&label_buffer)
                    .trim_end_matches('\0')
                    .to_owned()
            });

            out.push(DriveInfo {
                root: PathBuf::from(&root),
                label: label.filter(|text| !text.is_empty()),
                total_bytes: total,
                free_bytes: free_to_caller,
                kind,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn used_bytes_never_goes_negative() {
        let drive = DriveInfo {
            root: PathBuf::from("C:\\"),
            label: None,
            // A reported free figure larger than the total is nonsense, but a
            // network drive can report one; saturating means the bar clamps
            // rather than wrapping to 18 exabytes.
            total_bytes: 100,
            free_bytes: 250,
            kind: DriveKind::Fixed,
        };
        assert_eq!(drive.used_bytes(), 0);
    }

    #[test]
    fn a_path_is_matched_to_its_drive() {
        let volumes = StubVolumes::new()
            .with_drive("C:\\", 1_000, 400)
            .with_drive("D:\\", 2_000, 1_000);

        let found = volumes
            .for_path(Path::new("D:\\Games\\thing.exe"))
            .expect("D: is in the list");
        assert_eq!(found.root, PathBuf::from("D:\\"));
        assert_eq!(found.used_bytes(), 1_000);
    }

    #[test]
    fn a_path_on_an_unknown_drive_matches_nothing() {
        let volumes = StubVolumes::new().with_drive("C:\\", 1_000, 400);
        assert!(volumes.for_path(Path::new("Z:\\somewhere")).is_none());
    }

    /// The real enumeration must not panic and must report drives that make
    /// sense: this is the one place the app asks the machine about its disks.
    #[test]
    fn the_real_enumeration_reports_usable_drives() {
        for drive in SystemVolumes.drives() {
            assert!(
                drive.total_bytes > 0,
                "`{}` was reported with no capacity",
                drive.root.display()
            );
            assert!(
                drive.free_bytes <= drive.total_bytes.saturating_mul(2),
                "`{}` reported more free than plausible",
                drive.root.display()
            );
        }
    }
}
