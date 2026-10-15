//! Legacy panels (module spec 10): one screen of shortcuts to the classic
//! Windows windows that newer Settings pages hide.
//!
//! The catalogue is fixed here. The UI only ever sends an id; every target is
//! a file name joined onto `%SystemRoot%\System32`, so no path or command line
//! crosses IPC. WinKnife changes nothing itself: it asks the shell to open
//! Windows' own window, through [`PanelLauncher`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::platform::PanelLauncher;

/// Where a panel sits on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelGroup {
    /// System settings and services.
    System,
    /// Hardware & devices.
    Hardware,
    /// Network.
    Network,
    /// Apps & accounts.
    Apps,
    /// Diagnostics.
    Diagnostics,
}

/// How a panel is opened. Every name is a bare file name in System32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// `control.exe <System32>\name`.
    Cpl(&'static str),
    /// `mmc.exe <System32>\name`.
    Msc(&'static str),
    /// `<System32>\name`, no arguments.
    Exe(&'static str),
}

const CONTROL_EXE: &str = "control.exe";
const MMC_EXE: &str = "mmc.exe";

/// One catalogue entry.
#[derive(Debug, Clone, Copy)]
struct Panel {
    id: &'static str,
    group: PanelGroup,
    name: &'static str,
    description: &'static str,
    target: Target,
}

const CATALOGUE: &[Panel] = &[
    Panel {
        id: "system-properties",
        group: PanelGroup::System,
        name: "System Properties",
        description: "Computer name, virtual memory, startup and recovery, remote access.",
        target: Target::Cpl("sysdm.cpl"),
    },
    Panel {
        id: "power-options",
        group: PanelGroup::System,
        name: "Power Options",
        description: "Classic power plans and what the power button and lid do.",
        target: Target::Cpl("powercfg.cpl"),
    },
    Panel {
        id: "system-configuration",
        group: PanelGroup::System,
        name: "System Configuration",
        description: "Boot options and safe mode (msconfig).",
        target: Target::Exe("msconfig.exe"),
    },
    Panel {
        id: "services",
        group: PanelGroup::System,
        name: "Services",
        description: "Every Windows service, its state and how it starts.",
        target: Target::Msc("services.msc"),
    },
    Panel {
        id: "task-scheduler",
        group: PanelGroup::System,
        name: "Task Scheduler",
        description: "Tasks Windows and your apps run on a schedule or at sign-in.",
        target: Target::Msc("taskschd.msc"),
    },
    Panel {
        id: "computer-management",
        group: PanelGroup::System,
        name: "Computer Management",
        description: "Disks, services, event logs and shared folders in one window.",
        target: Target::Msc("compmgmt.msc"),
    },
    Panel {
        id: "region",
        group: PanelGroup::System,
        name: "Region",
        description: "Date, time and number formats, and the system locale.",
        target: Target::Cpl("intl.cpl"),
    },
    Panel {
        id: "date-and-time",
        group: PanelGroup::System,
        name: "Date and Time",
        description: "Time zone, extra clocks and internet time sync.",
        target: Target::Cpl("timedate.cpl"),
    },
    Panel {
        id: "control-panel",
        group: PanelGroup::System,
        name: "Control Panel",
        description: "The classic Control Panel home.",
        target: Target::Exe(CONTROL_EXE),
    },
    Panel {
        id: "device-manager",
        group: PanelGroup::Hardware,
        name: "Device Manager",
        description: "Every device, its driver, and the ones that are not working.",
        target: Target::Msc("devmgmt.msc"),
    },
    Panel {
        id: "disk-management",
        group: PanelGroup::Hardware,
        name: "Disk Management",
        description: "Partitions, volumes and drive letters.",
        target: Target::Msc("diskmgmt.msc"),
    },
    Panel {
        id: "sound",
        group: PanelGroup::Hardware,
        name: "Sound",
        description: "Classic playback and recording devices, and system sounds.",
        target: Target::Cpl("mmsys.cpl"),
    },
    Panel {
        id: "mouse",
        group: PanelGroup::Hardware,
        name: "Mouse",
        description: "Pointer speed, buttons, pointers and wheel.",
        target: Target::Cpl("main.cpl"),
    },
    Panel {
        id: "disk-cleanup",
        group: PanelGroup::Hardware,
        name: "Disk Cleanup",
        description: "Windows' own cleaner, including Windows Update leftovers.",
        target: Target::Exe("cleanmgr.exe"),
    },
    Panel {
        id: "network-connections",
        group: PanelGroup::Network,
        name: "Network Connections",
        description: "Network adapters: enable, disable, IP and DNS settings.",
        target: Target::Cpl("ncpa.cpl"),
    },
    Panel {
        id: "internet-options",
        group: PanelGroup::Network,
        name: "Internet Options",
        description: "Proxy, certificates and other system-wide internet settings.",
        target: Target::Cpl("inetcpl.cpl"),
    },
    Panel {
        id: "firewall",
        group: PanelGroup::Network,
        name: "Windows Defender Firewall",
        description: "Classic firewall settings and allowed apps.",
        target: Target::Cpl("firewall.cpl"),
    },
    Panel {
        id: "programs-and-features",
        group: PanelGroup::Apps,
        name: "Programs and Features",
        description: "Installed programs, updates and optional Windows features.",
        target: Target::Cpl("appwiz.cpl"),
    },
    Panel {
        id: "user-accounts",
        group: PanelGroup::Apps,
        name: "User Accounts",
        description: "Users on this computer and automatic sign-in (netplwiz).",
        target: Target::Exe("netplwiz.exe"),
    },
    Panel {
        id: "event-viewer",
        group: PanelGroup::Diagnostics,
        name: "Event Viewer",
        description: "Windows' logs: crashes, errors and warnings.",
        target: Target::Msc("eventvwr.msc"),
    },
    Panel {
        id: "resource-monitor",
        group: PanelGroup::Diagnostics,
        name: "Resource Monitor",
        description: "Live CPU, memory, disk and network use per process.",
        target: Target::Exe("resmon.exe"),
    },
    Panel {
        id: "system-information",
        group: PanelGroup::Diagnostics,
        name: "System Information",
        description: "Hardware, drivers and Windows details in one report.",
        target: Target::Exe("msinfo32.exe"),
    },
];

/// What the page shows for one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelInfo {
    /// Stable id; the only thing the UI sends back.
    pub id: String,
    /// Section on the page.
    pub group: PanelGroup,
    /// Window name.
    pub name: String,
    /// What the window is for, in one sentence.
    pub description: String,
    /// False when the window is not on this edition of Windows.
    pub available: bool,
}

/// The program to start and its single argument.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Launch {
    file: PathBuf,
    argument: Option<PathBuf>,
}

fn launch_for(target: Target, system32: &Path) -> Launch {
    match target {
        Target::Cpl(name) => Launch {
            file: system32.join(CONTROL_EXE),
            argument: Some(system32.join(name)),
        },
        Target::Msc(name) => Launch {
            file: system32.join(MMC_EXE),
            argument: Some(system32.join(name)),
        },
        Target::Exe(name) => Launch {
            file: system32.join(name),
            argument: None,
        },
    }
}

/// The app side of the module: list and open.
pub struct LegacyPanelsService {
    launcher: Arc<dyn PanelLauncher>,
    /// `%SystemRoot%\System32`, or `None` when `SystemRoot` is not set (every
    /// entry is then unavailable rather than guessed).
    system32: Option<PathBuf>,
}

impl LegacyPanelsService {
    /// Compose the service. `system_root` is `%SystemRoot%`.
    pub fn new(launcher: Arc<dyn PanelLauncher>, system_root: Option<PathBuf>) -> Self {
        Self {
            launcher,
            system32: system_root.map(|root| root.join("System32")),
        }
    }

    fn launch_if_available(&self, panel: &Panel) -> Option<Launch> {
        let launch = launch_for(panel.target, self.system32.as_deref()?);
        let present = self.launcher.exists(&launch.file)
            && launch
                .argument
                .as_deref()
                .is_none_or(|argument| self.launcher.exists(argument));
        present.then_some(launch)
    }

    /// The whole catalogue, in page order. Read-only.
    pub fn list(&self) -> Vec<PanelInfo> {
        CATALOGUE
            .iter()
            .map(|panel| PanelInfo {
                id: panel.id.to_owned(),
                group: panel.group,
                name: panel.name.to_owned(),
                description: panel.description.to_owned(),
                available: self.launch_if_available(panel).is_some(),
            })
            .collect()
    }

    /// Open the window with this id.
    ///
    /// # Errors
    /// Returns an error for an id that is not in the catalogue, for a window
    /// that is not on this edition of Windows, or when the shell refused.
    pub fn open(&self, id: &str) -> Result<()> {
        let panel = CATALOGUE
            .iter()
            .find(|panel| panel.id == id)
            .ok_or_else(|| CoreError::UnknownLegacyPanel(id.to_owned()))?;
        let launch = self.launch_if_available(panel).ok_or_else(|| {
            CoreError::LegacyPanel(format!("{} is not on this edition of Windows", panel.name))
        })?;
        tracing::info!(id, "opening legacy panel");
        self.launcher.open(&launch.file, launch.argument.as_deref())
    }
}

impl std::fmt::Debug for LegacyPanelsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegacyPanelsService")
            .field("system32", &self.system32)
            .finish_non_exhaustive()
    }
}

/// Registry entry. No scan, no findings: a list of buttons.
#[derive(Debug, Default)]
pub struct LegacyPanelsModule;

impl ToolModule for LegacyPanelsModule {
    fn id(&self) -> &'static str {
        "legacy-panels"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Legacy Panels".to_owned(),
            description: "Open the classic Windows windows that Settings hides.".to_owned(),
            icon: "panels".to_owned(),
            capabilities: ModuleCapabilities {
                has_scan: false,
                has_execute: false,
                has_settings: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::{FakePanelLauncher, PanelLaunch, SystemPanelLauncher};
    use std::collections::HashSet;

    const ROOT: &str = r"C:\Windows";

    fn system32() -> PathBuf {
        PathBuf::from(ROOT).join("System32")
    }

    fn service(fake: &Arc<FakePanelLauncher>) -> LegacyPanelsService {
        LegacyPanelsService::new(Arc::clone(fake) as Arc<dyn PanelLauncher>, Some(ROOT.into()))
    }

    #[test]
    fn ids_are_unique_kebab_case() {
        let mut seen = HashSet::new();
        for panel in CATALOGUE {
            assert!(seen.insert(panel.id), "duplicate id {}", panel.id);
            assert!(
                !panel.id.is_empty()
                    && panel
                        .id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                    && !panel.id.starts_with('-')
                    && !panel.id.ends_with('-'),
                "id `{}` is not kebab-case",
                panel.id
            );
        }
    }

    #[test]
    fn every_entry_has_a_name_and_a_description() {
        for panel in CATALOGUE {
            assert!(!panel.name.trim().is_empty(), "{} has no name", panel.id);
            assert!(
                panel.description.trim().ends_with('.'),
                "{} needs a one-sentence description",
                panel.id
            );
        }
    }

    #[test]
    fn every_target_resolves_under_system32() {
        let system32 = system32();
        for panel in CATALOGUE {
            let name = match panel.target {
                Target::Cpl(name) | Target::Msc(name) | Target::Exe(name) => name,
            };
            assert!(
                !name.contains(['\\', '/', ':', '"', ' ']),
                "{}: `{name}` must be a bare file name",
                panel.id
            );
            let expected_ext = match panel.target {
                Target::Cpl(_) => ".cpl",
                Target::Msc(_) => ".msc",
                Target::Exe(_) => ".exe",
            };
            assert!(name.ends_with(expected_ext), "{}: `{name}`", panel.id);
            let launch = launch_for(panel.target, &system32);
            assert_eq!(launch.file.parent(), Some(system32.as_path()));
            if let Some(argument) = launch.argument {
                assert_eq!(argument.parent(), Some(system32.as_path()));
            }
        }
    }

    #[test]
    fn opening_a_known_id_launches_the_expected_file_and_argument() {
        let fake = Arc::new(FakePanelLauncher::default());
        let service = service(&fake);
        service.open("network-connections").expect("open cpl");
        service.open("device-manager").expect("open msc");
        service.open("resource-monitor").expect("open exe");
        let s = system32();
        assert_eq!(
            fake.launches(),
            vec![
                PanelLaunch {
                    file: s.join("control.exe"),
                    argument: Some(s.join("ncpa.cpl")),
                },
                PanelLaunch {
                    file: s.join("mmc.exe"),
                    argument: Some(s.join("devmgmt.msc")),
                },
                PanelLaunch {
                    file: s.join("resmon.exe"),
                    argument: None,
                },
            ]
        );
    }

    #[test]
    fn an_unknown_id_errors_and_launches_nothing() {
        let fake = Arc::new(FakePanelLauncher::default());
        let service = service(&fake);
        for id in ["", "regedit", r"C:\Windows\System32\cmd.exe", "..\\x"] {
            let err = service.open(id).expect_err("unknown id");
            assert!(matches!(err, CoreError::UnknownLegacyPanel(_)), "{id}");
        }
        assert!(fake.launches().is_empty());
    }

    #[test]
    fn a_missing_file_is_unavailable_and_refuses_to_open() {
        let fake = Arc::new(FakePanelLauncher::with_missing([
            system32().join("msconfig.exe"),
            system32().join("mmc.exe"),
        ]));
        let service = service(&fake);
        let list = service.list();
        let available = |id: &str| list.iter().find(|p| p.id == id).expect(id).available;
        assert!(!available("system-configuration"));
        // A missing host program takes every snap-in with it.
        assert!(!available("services"));
        assert!(!available("device-manager"));
        assert!(available("network-connections"));

        assert!(matches!(
            service.open("system-configuration"),
            Err(CoreError::LegacyPanel(_))
        ));
        assert!(matches!(
            service.open("services"),
            Err(CoreError::LegacyPanel(_))
        ));
        assert!(fake.launches().is_empty());
    }

    #[test]
    fn without_system_root_nothing_is_available() {
        let fake = Arc::new(FakePanelLauncher::default());
        let service = LegacyPanelsService::new(Arc::clone(&fake) as Arc<dyn PanelLauncher>, None);
        assert!(service.list().iter().all(|panel| !panel.available));
        assert!(service.open("services").is_err());
        assert!(fake.launches().is_empty());
    }

    #[test]
    fn the_list_follows_the_catalogue() {
        let fake = Arc::new(FakePanelLauncher::default());
        let list = service(&fake).list();
        assert_eq!(list.len(), CATALOGUE.len());
        assert!(list.iter().all(|panel| panel.available));
        let json = serde_json::to_value(&list[0]).expect("serialize");
        assert_eq!(json["group"], "system");
        assert_eq!(json["id"], "system-properties");
    }

    #[test]
    fn a_test_build_cannot_launch_for_real() {
        let service = LegacyPanelsService::new(Arc::new(SystemPanelLauncher), Some(ROOT.into()));
        if service.list().iter().any(|panel| panel.id == "control-panel" && panel.available) {
            assert!(matches!(
                service.open("control-panel"),
                Err(CoreError::LegacyPanel(_))
            ));
        }
    }
}
