//! Network (C14, K-023): which process listens on or is connected to which
//! port. Read-only. The one action, flushing the DNS cache, is
//! `hosts_flush_dns` and lives with the hosts module.
//!
//! `core/platform/network.rs` reads the OS tables; this file turns them into
//! rows with a plain-language reason and sorts them.

use std::net::IpAddr;
use std::path::Path;

use serde::Serialize;

use crate::core::error::Result;
use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::platform::{read_sockets, Protocol, RawSocket, TcpState};

/// Where a socket can be reached from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reach {
    /// `0.0.0.0` / `::`: every network interface, so other machines can try it.
    AllInterfaces,
    /// `127.0.0.1` / `::1`: this PC only.
    LocalOnly,
    /// One specific interface address.
    OneInterface,
}

/// Listening sockets first, then live connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RowKind {
    Listening,
    Connection,
}

/// One line on the Network page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortRow {
    pub protocol: &'static str,
    pub kind: RowKind,
    /// `LISTEN`, `ESTABLISHED`, ... ; `None` for UDP.
    pub state: Option<&'static str>,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: Option<String>,
    pub remote_port: Option<u16>,
    pub reach: Reach,
    pub pid: u32,
    pub process_name: String,
    pub exe_path: Option<String>,
    /// Why this row looks the way it does, in a sentence.
    pub reason: String,
}

const IDLE_PID: u32 = 0;
const SYSTEM_PID: u32 = 4;

/// What a bound address means for who can connect.
pub fn reach_of(addr: IpAddr) -> Reach {
    if addr.is_unspecified() {
        Reach::AllInterfaces
    } else if addr.is_loopback() {
        Reach::LocalOnly
    } else {
        Reach::OneInterface
    }
}

fn state_label(state: TcpState) -> &'static str {
    match state {
        TcpState::Closed => "CLOSED",
        TcpState::Listen => "LISTEN",
        TcpState::SynSent => "SYN_SENT",
        TcpState::SynReceived => "SYN_RECEIVED",
        TcpState::Established => "ESTABLISHED",
        TcpState::FinWait1 => "FIN_WAIT_1",
        TcpState::FinWait2 => "FIN_WAIT_2",
        TcpState::CloseWait => "CLOSE_WAIT",
        TcpState::Closing => "CLOSING",
        TcpState::LastAck => "LAST_ACK",
        TcpState::TimeWait => "TIME_WAIT",
        TcpState::DeleteTcb => "DELETE_TCB",
    }
}

fn process_name(pid: u32, exe: Option<&Path>) -> String {
    match pid {
        IDLE_PID => "System Idle Process".to_owned(),
        SYSTEM_PID => "System".to_owned(),
        _ => exe
            .and_then(Path::file_name)
            .map_or_else(|| "Unknown".to_owned(), |n| n.to_string_lossy().into_owned()),
    }
}

/// Under `%SystemRoot%` (case-insensitive, either slash).
fn is_windows_file(exe: &Path, system_root: Option<&str>) -> bool {
    let Some(root) = system_root else {
        return false;
    };
    let norm = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let root = norm(root);
    norm(&exe.to_string_lossy()).starts_with(&format!("{root}\\"))
}

fn who(pid: u32, exe: Option<&Path>, system_root: Option<&str>) -> &'static str {
    if pid <= SYSTEM_PID {
        "System process"
    } else {
        match exe {
            Some(path) if is_windows_file(path, system_root) => "Windows component",
            Some(_) => "Program",
            None => "A process Windows keeps private (protected, elevated or already gone)",
        }
    }
}

fn reach_text(reach: Reach) -> &'static str {
    match reach {
        Reach::AllInterfaces => "reachable from every network this PC is on",
        Reach::LocalOnly => "reachable from this PC only",
        Reach::OneInterface => "bound to one network address",
    }
}

fn reason_for(who: &str, reach: Reach, kind: RowKind) -> String {
    match kind {
        RowKind::Listening => format!("{who}, waiting for connections: {}.", reach_text(reach)),
        RowKind::Connection => format!("{who}, with a connection open right now."),
    }
}

/// Turns the OS rows into display rows. Pure, so it is unit tested.
pub fn rows_from(sockets: &[RawSocket], system_root: Option<&str>) -> Vec<PortRow> {
    let mut rows: Vec<PortRow> = sockets
        .iter()
        .filter_map(|socket| {
            // TCP in states other than LISTEN/ESTABLISHED (TIME_WAIT and
            // friends) is noise for "who uses this port".
            let kind = match (socket.protocol, socket.state) {
                (Protocol::Udp, _) | (Protocol::Tcp, Some(TcpState::Listen)) => RowKind::Listening,
                (Protocol::Tcp, Some(TcpState::Established)) => RowKind::Connection,
                _ => return None,
            };
            let reach = reach_of(socket.local);
            let exe = socket.exe.as_deref();
            let reason = reason_for(who(socket.pid, exe, system_root), reach, kind);
            Some(PortRow {
                protocol: match socket.protocol {
                    Protocol::Tcp => "TCP",
                    Protocol::Udp => "UDP",
                },
                kind,
                state: socket.state.map(state_label),
                local_address: socket.local.to_string(),
                local_port: socket.local_port,
                remote_address: socket.remote.map(|a| a.to_string()),
                remote_port: socket.remote.map(|_| socket.remote_port),
                reach,
                pid: socket.pid,
                process_name: process_name(socket.pid, exe),
                exe_path: exe.map(|p| p.to_string_lossy().into_owned()),
                reason,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        (a.kind as u8, a.local_port, a.protocol, a.pid)
            .cmp(&(b.kind as u8, b.local_port, b.protocol, b.pid))
    });
    rows
}

/// Reads the live tables.
///
/// # Errors
/// Returns an error when the OS tables cannot be read.
pub fn list() -> Result<Vec<PortRow>> {
    let system_root = std::env::var("SystemRoot").ok();
    Ok(rows_from(&read_sockets()?, system_root.as_deref()))
}

/// Registry entry. No scan, no findings: a read-only list page.
#[derive(Debug, Default)]
pub struct NetworkModule;

impl ToolModule for NetworkModule {
    fn id(&self) -> &'static str {
        "network"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Network".to_owned(),
            description: "See which program uses which port, and flush the DNS cache.".to_owned(),
            icon: "network".to_owned(),
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
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::path::PathBuf;

    fn socket(protocol: Protocol, state: Option<TcpState>, local: IpAddr, port: u16) -> RawSocket {
        RawSocket {
            protocol,
            local,
            local_port: port,
            remote: (protocol == Protocol::Tcp).then_some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            remote_port: 0,
            state,
            pid: 1234,
            exe: Some(PathBuf::from(r"C:\Tools\server.exe")),
        }
    }

    const ROOT: Option<&str> = Some(r"C:\Windows");

    #[test]
    fn reach_separates_all_loopback_and_one_interface() {
        assert_eq!(reach_of(IpAddr::V4(Ipv4Addr::UNSPECIFIED)), Reach::AllInterfaces);
        assert_eq!(reach_of(IpAddr::V6(Ipv6Addr::UNSPECIFIED)), Reach::AllInterfaces);
        assert_eq!(reach_of(IpAddr::V4(Ipv4Addr::LOCALHOST)), Reach::LocalOnly);
        assert_eq!(reach_of(IpAddr::V6(Ipv6Addr::LOCALHOST)), Reach::LocalOnly);
        assert_eq!(reach_of(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))), Reach::OneInterface);
    }

    #[test]
    fn a_listener_on_every_interface_says_so() {
        let rows = rows_from(
            &[socket(Protocol::Tcp, Some(TcpState::Listen), IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8080)],
            ROOT,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, RowKind::Listening);
        assert_eq!(rows[0].process_name, "server.exe");
        assert!(rows[0].reason.contains("every network"), "{}", rows[0].reason);
        assert!(rows[0].reason.starts_with("Program,"), "{}", rows[0].reason);
    }

    #[test]
    fn a_localhost_listener_says_this_pc_only() {
        let rows = rows_from(
            &[socket(Protocol::Tcp, Some(TcpState::Listen), IpAddr::V4(Ipv4Addr::LOCALHOST), 5432)],
            ROOT,
        );
        assert!(rows[0].reason.contains("this PC only"), "{}", rows[0].reason);
    }

    #[test]
    fn system_and_windows_processes_are_named_as_such() {
        let mut kernel = socket(Protocol::Tcp, Some(TcpState::Listen), IpAddr::V4(Ipv4Addr::UNSPECIFIED), 445);
        kernel.pid = SYSTEM_PID;
        kernel.exe = None;
        let mut svchost = socket(Protocol::Tcp, Some(TcpState::Listen), IpAddr::V4(Ipv4Addr::UNSPECIFIED), 135);
        svchost.exe = Some(PathBuf::from(r"c:\windows\system32\svchost.exe"));
        let rows = rows_from(&[kernel, svchost], ROOT);
        let by_port = |p| rows.iter().find(|r| r.local_port == p).unwrap();
        assert_eq!(by_port(445).process_name, "System");
        assert!(by_port(445).reason.starts_with("System process"));
        assert!(by_port(135).reason.starts_with("Windows component"));
    }

    #[test]
    fn a_process_that_cannot_be_opened_is_not_called_a_program() {
        let mut hidden = socket(Protocol::Udp, None, IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5353);
        hidden.exe = None;
        let rows = rows_from(&[hidden], ROOT);
        assert_eq!(rows[0].process_name, "Unknown");
        assert!(rows[0].reason.contains("keeps private"), "{}", rows[0].reason);
        assert!(!rows[0].reason.starts_with("Program"), "{}", rows[0].reason);
    }

    #[test]
    fn only_listeners_udp_and_established_connections_are_kept() {
        let any = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        let rows = rows_from(
            &[
                socket(Protocol::Tcp, Some(TcpState::TimeWait), any, 1),
                socket(Protocol::Tcp, Some(TcpState::Established), any, 2),
                socket(Protocol::Tcp, Some(TcpState::Listen), any, 3),
                socket(Protocol::Udp, None, any, 4),
            ],
            ROOT,
        );
        let ports: Vec<u16> = rows.iter().map(|r| r.local_port).collect();
        // Listeners (and UDP) first by port, then the connection.
        assert_eq!(ports, vec![3, 4, 2]);
    }

    #[test]
    fn windows_path_check_ignores_case_and_slashes_and_needs_a_separator() {
        assert!(is_windows_file(Path::new("C:/WINDOWS/System32/x.exe"), ROOT));
        assert!(!is_windows_file(Path::new(r"C:\WindowsApps\x.exe"), ROOT));
        assert!(!is_windows_file(Path::new(r"C:\Windows\x.exe"), None));
    }
}
