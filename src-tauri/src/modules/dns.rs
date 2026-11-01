//! DNS switcher (#16): list the active adapters with their DNS servers, switch
//! to a preset, and put the previous servers back.
//!
//! Reading and writing go through Windows PowerShell (`Get-`/`Set-DnsClientServerAddress`)
//! behind [`DnsBackend`], so every rule here is tested without touching the PC.
//! Nothing here makes a network request. Each change records what the adapter
//! had before in `dns-history.json` (app data); revert pops it.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::platform::{is_elevated, real_os_effects_armed, EnvResolver, SystemEnv};

const HISTORY_FILE: &str = "dns-history.json";
/// Oldest changes are dropped past this many.
const HISTORY_MAX: usize = 50;
/// `CREATE_NO_WINDOW`: PowerShell must not flash a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Lists the up adapters. DHCP-assigned DNS has an empty `NameServer` in the
/// adapter's registry key, which the cmdlet alone does not show.
const LIST_SCRIPT: &str = r#"$r = @(Get-NetAdapter | Where-Object Status -eq 'Up' | ForEach-Object {
  $s = @((Get-DnsClientServerAddress -InterfaceIndex $_.ifIndex -AddressFamily IPv4).ServerAddresses)
  $k = "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\$($_.InterfaceGuid)"
  $n = (Get-ItemProperty $k -ErrorAction SilentlyContinue).NameServer
  [pscustomobject]@{ index = [int]$_.ifIndex; name = [string]$_.Name; servers = $s; automatic = [string]::IsNullOrEmpty($n) }
})
ConvertTo-Json -InputObject $r -Compress"#;

/// A DNS choice on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    Cloudflare,
    Google,
    Quad9,
    /// Let the router / DHCP hand out DNS again.
    Automatic,
}

impl Preset {
    /// The servers to set; empty means automatic (DHCP).
    pub fn servers(self) -> Vec<String> {
        let list: &[&str] = match self {
            Self::Cloudflare => &["1.1.1.1", "1.0.0.1"],
            Self::Google => &["8.8.8.8", "8.8.4.4"],
            Self::Quad9 => &["9.9.9.9", "149.112.112.112"],
            Self::Automatic => &[],
        };
        list.iter().map(|s| (*s).to_owned()).collect()
    }
}

/// One network adapter that is up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Adapter {
    pub index: u32,
    pub name: String,
    /// IPv4 DNS servers in use right now.
    pub servers: Vec<String>,
    /// True when they come from DHCP rather than being typed in.
    pub automatic: bool,
}

/// What an adapter had before a change; enough to undo it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsChange {
    pub index: u32,
    pub name: String,
    /// Empty together with `automatic` means it was on DHCP.
    pub previous: Vec<String>,
    pub previous_automatic: bool,
}

/// The adapters plus the changes that can still be reverted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsState {
    pub adapters: Vec<Adapter>,
    pub history: Vec<DnsChange>,
    pub elevated: bool,
}

/// The OS side, replaced in tests.
pub trait DnsBackend: Send + Sync {
    fn adapters(&self) -> Result<Vec<Adapter>>;
    /// Empty `servers` puts the adapter back on DHCP.
    fn set(&self, index: u32, servers: &[String]) -> Result<()>;
    fn is_elevated(&self) -> bool;
}

/// Parse the script's JSON: one object or an array of them.
///
/// # Errors
/// Returns an error when the text is not the expected JSON.
pub fn parse_adapters(json: &str) -> Result<Vec<Adapter>> {
    let text = json.trim();
    if text.is_empty() || text == "null" {
        return Ok(Vec::new());
    }
    let bad = |err: serde_json::Error| CoreError::Network(format!("unreadable adapter list: {err}"));
    if text.starts_with('[') {
        serde_json::from_str(text).map_err(bad)
    } else {
        serde_json::from_str::<Adapter>(text).map(|a| vec![a]).map_err(bad)
    }
}

/// Servers must be plain IPv4 addresses; they end up in a PowerShell command.
fn check_servers(servers: &[String]) -> Result<()> {
    for s in servers {
        s.parse::<Ipv4Addr>()
            .map_err(|_| CoreError::Network(format!("`{s}` is not an IPv4 address")))?;
    }
    Ok(())
}

/// The script for one change. Only a number and validated addresses go in.
fn set_script(index: u32, servers: &[String]) -> String {
    if servers.is_empty() {
        format!("Set-DnsClientServerAddress -InterfaceIndex {index} -ResetServerAddresses")
    } else {
        let list = servers.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",");
        format!("Set-DnsClientServerAddress -InterfaceIndex {index} -ServerAddresses {list}")
    }
}

/// Real backend: Windows PowerShell by full path.
pub struct SystemDns;

impl SystemDns {
    fn powershell(script: &str) -> Result<String> {
        if !real_os_effects_armed() {
            return Err(CoreError::Network("this build must never run PowerShell".to_owned()));
        }
        let root = SystemEnv
            .var("SystemRoot")
            .ok_or_else(|| CoreError::Network("%SystemRoot% is not set".to_owned()))?;
        let exe: PathBuf = Path::new(&root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let mut command = std::process::Command::new(exe);
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, CREATE_NO_WINDOW);
        let output = command.output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(CoreError::Network(format!(
                "PowerShell failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }
}

impl DnsBackend for SystemDns {
    fn adapters(&self) -> Result<Vec<Adapter>> {
        parse_adapters(&Self::powershell(LIST_SCRIPT)?)
    }

    fn set(&self, index: u32, servers: &[String]) -> Result<()> {
        check_servers(servers)?;
        Self::powershell(&set_script(index, servers)).map(|_| ())
    }

    fn is_elevated(&self) -> bool {
        is_elevated()
    }
}

/// Applies presets and keeps the undo list.
pub struct DnsService {
    backend: Arc<dyn DnsBackend>,
    history_path: PathBuf,
    // Serialises read-modify-write of the history file.
    lock: Mutex<()>,
}

impl DnsService {
    pub fn new(backend: Arc<dyn DnsBackend>, app_data_dir: &Path) -> Self {
        Self {
            backend,
            history_path: app_data_dir.join(HISTORY_FILE),
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Vec<DnsChange> {
        std::fs::read_to_string(&self.history_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, history: &[DnsChange]) -> Result<()> {
        let text = serde_json::to_string_pretty(history)
            .map_err(|err| CoreError::Network(format!("cannot save DNS history: {err}")))?;
        std::fs::write(&self.history_path, text)?;
        Ok(())
    }

    fn require_admin(&self) -> Result<()> {
        if self.backend.is_elevated() {
            Ok(())
        } else {
            Err(CoreError::NeedsAdmin("changing DNS servers needs administrator rights".to_owned()))
        }
    }

    /// Adapters now, plus what can be reverted.
    ///
    /// # Errors
    /// Returns an error when the adapters cannot be read.
    pub fn state(&self) -> Result<DnsState> {
        let _guard = self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(DnsState {
            adapters: self.backend.adapters()?,
            history: self.load(),
            elevated: self.backend.is_elevated(),
        })
    }

    /// Switch `index` to `preset`, remembering the old servers.
    ///
    /// # Errors
    /// `needs_admin` when unelevated, or an error when the adapter is gone or the change fails.
    pub fn apply(&self, index: u32, preset: Preset) -> Result<DnsChange> {
        self.require_admin()?;
        let _guard = self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let adapter = self
            .backend
            .adapters()?
            .into_iter()
            .find(|a| a.index == index)
            .ok_or_else(|| CoreError::Network("that network adapter is no longer up".to_owned()))?;
        let change = DnsChange {
            index,
            name: adapter.name,
            previous: adapter.servers,
            previous_automatic: adapter.automatic,
        };
        self.backend.set(index, &preset.servers())?;
        let mut history = self.load();
        history.push(change.clone());
        let excess = history.len().saturating_sub(HISTORY_MAX);
        history.drain(..excess);
        self.save(&history)?;
        Ok(change)
    }

    /// Undo the newest change for `index`.
    ///
    /// # Errors
    /// `needs_admin` when unelevated, or an error when nothing is recorded or the change fails.
    pub fn revert(&self, index: u32) -> Result<DnsChange> {
        self.require_admin()?;
        let _guard = self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut history = self.load();
        let at = history
            .iter()
            .rposition(|c| c.index == index)
            .ok_or_else(|| CoreError::Network("no earlier DNS setting is recorded".to_owned()))?;
        let change = history[at].clone();
        let servers = if change.previous_automatic { Vec::new() } else { change.previous.clone() };
        self.backend.set(index, &servers)?;
        history.remove(at);
        self.save(&history)?;
        Ok(change)
    }
}

/// Registry entry.
#[derive(Default)]
pub struct DnsModule;

impl ToolModule for DnsModule {
    fn id(&self) -> &'static str {
        "dns"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "DNS switcher".to_owned(),
            description: "Switch your DNS servers to Cloudflare, Google or Quad9, and switch back.".to_owned(),
            icon: "dns".to_owned(),
            category: ModuleCategory::Network,
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

    struct Fake {
        adapters: Mutex<Vec<Adapter>>,
        elevated: bool,
    }

    impl DnsBackend for Fake {
        fn adapters(&self) -> Result<Vec<Adapter>> {
            Ok(self.adapters.lock().unwrap().clone())
        }
        fn set(&self, index: u32, servers: &[String]) -> Result<()> {
            let mut all = self.adapters.lock().unwrap();
            let a = all.iter_mut().find(|a| a.index == index).unwrap();
            a.automatic = servers.is_empty();
            a.servers = servers.to_vec();
            Ok(())
        }
        fn is_elevated(&self) -> bool {
            self.elevated
        }
    }

    fn eth(servers: &[&str], automatic: bool) -> Adapter {
        Adapter {
            index: 7,
            name: "Ethernet".to_owned(),
            servers: servers.iter().map(|s| (*s).to_owned()).collect(),
            automatic,
        }
    }

    fn service(adapter: Adapter, elevated: bool) -> (DnsService, Arc<Fake>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake { adapters: Mutex::new(vec![adapter]), elevated });
        (DnsService::new(fake.clone(), dir.path()), fake, dir)
    }

    #[test]
    fn presets_map_to_their_servers() {
        assert_eq!(Preset::Cloudflare.servers(), ["1.1.1.1", "1.0.0.1"]);
        assert_eq!(Preset::Google.servers(), ["8.8.8.8", "8.8.4.4"]);
        assert_eq!(Preset::Quad9.servers(), ["9.9.9.9", "149.112.112.112"]);
        assert!(Preset::Automatic.servers().is_empty());
    }

    #[test]
    fn parses_one_object_an_array_and_nothing() {
        let one = r#"{"index":7,"name":"Ethernet","servers":["192.168.1.1"],"automatic":true}"#;
        let many = format!("[{one},{one}]");
        assert_eq!(parse_adapters(one).unwrap().len(), 1);
        assert_eq!(parse_adapters(&many).unwrap().len(), 2);
        assert!(parse_adapters("  ").unwrap().is_empty());
        assert!(parse_adapters("null").unwrap().is_empty());
        assert!(parse_adapters("not json").is_err());
    }

    #[test]
    fn only_ipv4_addresses_reach_the_command() {
        assert!(check_servers(&["1.1.1.1".to_owned()]).is_ok());
        assert!(check_servers(&["1.1.1.1'; calc; '".to_owned()]).is_err());
        assert_eq!(
            set_script(7, &["1.1.1.1".to_owned(), "1.0.0.1".to_owned()]),
            "Set-DnsClientServerAddress -InterfaceIndex 7 -ServerAddresses '1.1.1.1','1.0.0.1'"
        );
        assert!(set_script(7, &[]).ends_with("-ResetServerAddresses"));
    }

    #[test]
    fn apply_needs_admin_and_changes_nothing_without_it() {
        let (svc, fake, _dir) = service(eth(&["192.168.1.1"], true), false);
        let err = svc.apply(7, Preset::Google).unwrap_err();
        assert!(matches!(err, CoreError::NeedsAdmin(_)));
        assert_eq!(fake.adapters.lock().unwrap()[0].servers, ["192.168.1.1"]);
        assert!(svc.state().unwrap().history.is_empty());
    }

    #[test]
    fn apply_records_the_previous_servers_and_revert_restores_them() {
        let (svc, fake, _dir) = service(eth(&["192.168.1.1"], false), true);
        svc.apply(7, Preset::Cloudflare).unwrap();
        assert_eq!(fake.adapters.lock().unwrap()[0].servers, ["1.1.1.1", "1.0.0.1"]);
        assert_eq!(svc.state().unwrap().history.len(), 1);

        svc.revert(7).unwrap();
        let after = fake.adapters.lock().unwrap()[0].clone();
        assert_eq!(after.servers, ["192.168.1.1"]);
        assert!(svc.state().unwrap().history.is_empty());
    }

    #[test]
    fn reverting_a_dhcp_adapter_goes_back_to_automatic() {
        let (svc, fake, _dir) = service(eth(&["192.168.1.1"], true), true);
        svc.apply(7, Preset::Quad9).unwrap();
        svc.revert(7).unwrap();
        assert!(fake.adapters.lock().unwrap()[0].automatic);
    }

    #[test]
    fn two_changes_revert_newest_first() {
        let (svc, fake, _dir) = service(eth(&["192.168.1.1"], false), true);
        svc.apply(7, Preset::Google).unwrap();
        svc.apply(7, Preset::Cloudflare).unwrap();
        svc.revert(7).unwrap();
        assert_eq!(fake.adapters.lock().unwrap()[0].servers, ["8.8.8.8", "8.8.4.4"]);
    }

    #[test]
    fn revert_without_history_or_unknown_adapter_is_an_error() {
        let (svc, _fake, _dir) = service(eth(&[], true), true);
        assert!(svc.revert(7).is_err());
        assert!(svc.apply(99, Preset::Google).is_err());
    }

    #[test]
    fn history_is_capped() {
        let (svc, _fake, _dir) = service(eth(&["192.168.1.1"], false), true);
        for _ in 0..HISTORY_MAX + 5 {
            svc.apply(7, Preset::Google).unwrap();
        }
        assert_eq!(svc.state().unwrap().history.len(), HISTORY_MAX);
    }
}
