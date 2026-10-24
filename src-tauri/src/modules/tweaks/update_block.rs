//! "Is something blocking Windows Update?" (C7). Read-only: this module only
//! ever calls [`Registry::value`]. It reports policy and service values other
//! tools leave behind; WinKnife never writes them, and the tweak fence keeps it
//! that way.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::core::platform::{Hive, RegValue, Registry};

const POLICY_WU: &str = r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate";
const POLICY_AU: &str = r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU";
const SERVICE_WUAUSERV: &str = r"SYSTEM\CurrentControlSet\Services\wuauserv";
const UX_SETTINGS: &str = r"SOFTWARE\Microsoft\WindowsUpdate\UX\Settings";
/// `Start` value of a disabled service.
const SERVICE_DISABLED: u32 = 4;

/// One value that stops or holds back Windows Update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBlock {
    /// Exact registry path, value name included.
    pub path: String,
    /// What the value holds, as text.
    pub value: String,
    pub reason: String,
}

/// What the check found. `errors` lists the values that could not be read, so
/// "nothing found" is never claimed for a key the check could not see.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub blocks: Vec<UpdateBlock>,
    pub errors: Vec<String>,
}

/// A DWORD that blocks updates when it equals `blocking`.
const DWORD_CHECKS: [(&str, &str, u32, &str); 5] = [
    (
        POLICY_AU,
        "NoAutoUpdate",
        1,
        "A policy turns automatic updates off.",
    ),
    (
        POLICY_WU,
        "DisableWindowsUpdateAccess",
        1,
        "A policy removes access to Windows Update.",
    ),
    (
        POLICY_WU,
        "DoNotConnectToWindowsUpdateInternetLocations",
        1,
        "A policy stops Windows from reaching Windows Update on the internet.",
    ),
    (
        POLICY_WU,
        "SetDisableUXWUAccess",
        1,
        "A policy hides the Windows Update settings page.",
    ),
    (
        SERVICE_WUAUSERV,
        "Start",
        SERVICE_DISABLED,
        "The Windows Update service is disabled.",
    ),
];

fn display(subkey: &str, name: &str) -> String {
    format!(r"HKLM\{subkey}\{name}")
}

/// Read the known blocking values. Never writes.
pub fn check(registry: &dyn Registry, now: DateTime<Utc>) -> UpdateStatus {
    let mut status = UpdateStatus::default();
    let mut read = |subkey: &str, name: &str| -> Option<RegValue> {
        registry
            .value(Hive::LocalMachine, subkey, name)
            .unwrap_or_else(|err| {
                status.errors.push(format!("{}: {err}", display(subkey, name)));
                None
            })
    };

    let mut blocks = Vec::new();
    for (subkey, name, blocking, reason) in DWORD_CHECKS {
        if let Some(number) = read(subkey, name).and_then(|value| value.as_dword()) {
            if number == blocking {
                blocks.push(UpdateBlock {
                    path: display(subkey, name),
                    value: number.to_string(),
                    reason: reason.to_owned(),
                });
            }
        }
    }
    if let Some(server) = read(POLICY_WU, "WUServer").and_then(|value| value.as_string()) {
        if !server.trim().is_empty() {
            blocks.push(UpdateBlock {
                path: display(POLICY_WU, "WUServer"),
                value: server,
                reason: "A policy points Windows Update at another server (often a company \
                         one), so updates come from there, not from Microsoft."
                    .to_owned(),
            });
        }
    }
    if let Some(text) =
        read(UX_SETTINGS, "PauseUpdatesExpiryTime").and_then(|value| value.as_string())
    {
        if let Ok(until) = DateTime::parse_from_rfc3339(text.trim()) {
            if until.with_timezone(&Utc) > now {
                blocks.push(UpdateBlock {
                    path: display(UX_SETTINGS, "PauseUpdatesExpiryTime"),
                    value: text.trim().to_owned(),
                    reason: format!(
                        "Updates are paused until {}. You can resume them in Settings.",
                        until.format("%Y-%m-%d")
                    ),
                });
            }
        }
    }
    status.blocks = blocks;
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::{MapRegistry, REG_TYPE_SZ};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-24T12:00:00Z")
            .expect("time")
            .with_timezone(&Utc)
    }

    #[test]
    fn a_clean_machine_reports_nothing_and_nothing_is_written() {
        let registry = MapRegistry::new();
        assert_eq!(check(&registry, now()), UpdateStatus::default());
        assert!(registry.writes().is_empty());
    }

    #[test]
    fn every_known_block_is_reported_with_a_reason_and_nothing_is_written() {
        let registry = MapRegistry::new()
            .with_key(
                Hive::LocalMachine,
                POLICY_AU,
                vec![RegValue::dword("NoAutoUpdate", 1)],
            )
            .with_key(
                Hive::LocalMachine,
                POLICY_WU,
                vec![
                    RegValue::dword("DisableWindowsUpdateAccess", 1),
                    RegValue::dword("DoNotConnectToWindowsUpdateInternetLocations", 1),
                    RegValue::dword("SetDisableUXWUAccess", 1),
                    RegValue::string("WUServer", REG_TYPE_SZ, "http://wsus.local"),
                ],
            )
            .with_key(
                Hive::LocalMachine,
                SERVICE_WUAUSERV,
                vec![RegValue::dword("Start", SERVICE_DISABLED)],
            )
            .with_key(
                Hive::LocalMachine,
                UX_SETTINGS,
                vec![RegValue::string(
                    "PauseUpdatesExpiryTime",
                    REG_TYPE_SZ,
                    "2026-11-01T00:00:00Z",
                )],
            );
        let status = check(&registry, now());
        assert_eq!(status.blocks.len(), 7, "{status:?}");
        assert!(status.errors.is_empty());
        assert!(status.blocks.iter().all(|block| !block.reason.is_empty()));
        assert!(registry.writes().is_empty());
    }

    #[test]
    fn harmless_values_are_not_reported() {
        let registry = MapRegistry::new()
            .with_key(
                Hive::LocalMachine,
                POLICY_AU,
                vec![RegValue::dword("NoAutoUpdate", 0)],
            )
            .with_key(
                Hive::LocalMachine,
                SERVICE_WUAUSERV,
                vec![RegValue::dword("Start", 3)],
            )
            .with_key(
                Hive::LocalMachine,
                UX_SETTINGS,
                // A pause that already ran out, and one Windows wrote oddly.
                vec![RegValue::string(
                    "PauseUpdatesExpiryTime",
                    REG_TYPE_SZ,
                    "2026-10-01T00:00:00Z",
                )],
            )
            .with_key(
                Hive::LocalMachine,
                POLICY_WU,
                vec![RegValue::string("WUServer", REG_TYPE_SZ, "  ")],
            );
        assert!(check(&registry, now()).blocks.is_empty());
    }
}
