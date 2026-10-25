//! Read-only view of the machine's TCP and UDP sockets (module C14, K-023).
//!
//! `GetExtendedTcpTable` / `GetExtendedUdpTable` return one row per socket with
//! the owning PID. This file only turns those rows into [`RawSocket`]; the
//! wording and classification live in `modules::network`. Nothing here writes.

use std::net::IpAddr;
use std::path::PathBuf;

use crate::core::error::{CoreError, Result};

/// Which table a socket came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
}

/// The TCP state machine, as `MIB_TCP_STATE` numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
    DeleteTcb,
}

impl TcpState {
    /// `None` for a number Windows has not defined.
    pub fn from_mib(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Closed,
            2 => Self::Listen,
            3 => Self::SynSent,
            4 => Self::SynReceived,
            5 => Self::Established,
            6 => Self::FinWait1,
            7 => Self::FinWait2,
            8 => Self::CloseWait,
            9 => Self::Closing,
            10 => Self::LastAck,
            11 => Self::TimeWait,
            12 => Self::DeleteTcb,
            _ => return None,
        })
    }
}

/// One socket as the OS reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSocket {
    pub protocol: Protocol,
    pub local: IpAddr,
    pub local_port: u16,
    /// `None` for UDP, which has no peer.
    pub remote: Option<IpAddr>,
    pub remote_port: u16,
    pub state: Option<TcpState>,
    pub pid: u32,
    /// Full image path; `None` when the process cannot be opened (protected,
    /// exited, or owned by another user).
    pub exe: Option<PathBuf>,
}

/// Every TCP and UDP socket (IPv4 and IPv6) right now.
///
/// # Errors
/// Returns an error when an IP Helper call fails, or off Windows.
#[cfg(windows)]
pub fn read_sockets() -> Result<Vec<RawSocket>> {
    imp::read_sockets()
}

/// # Errors
/// Always: there is no IP Helper off Windows.
#[cfg(not(windows))]
pub fn read_sockets() -> Result<Vec<RawSocket>> {
    Err(CoreError::Network("only available on Windows".to_owned()))
}

/// A port as `dwLocalPort` stores it: network byte order in the low 16 bits.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn port_from_net(raw: u32) -> u16 {
    u16::from_be((raw & 0xFFFF) as u16)
}

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::path::PathBuf;

    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_UDP6ROW_OWNER_PID, MIB_UDPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
    };
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::{port_from_net, CoreError, Protocol, RawSocket, Result, TcpState};

    /// The table can grow between the size query and the read.
    const ATTEMPTS: usize = 5;
    const MAX_IMAGE_PATH: usize = 32_768;

    pub(super) fn read_sockets() -> Result<Vec<RawSocket>> {
        let mut sockets = Vec::new();
        for family in [u32::from(AF_INET.0), u32::from(AF_INET6.0)] {
            let is_v6 = family == u32::from(AF_INET6.0);
            let tcp = table(|buf, size| {
                // SAFETY: `buf` is null or `size` bytes long; `size` is valid.
                unsafe { GetExtendedTcpTable(buf, size, false, family, TCP_TABLE_OWNER_PID_ALL, 0) }
            })?;
            sockets.extend(if is_v6 { tcp6(&tcp) } else { tcp4(&tcp) });
            let udp = table(|buf, size| {
                // SAFETY: as above.
                unsafe { GetExtendedUdpTable(buf, size, false, family, UDP_TABLE_OWNER_PID, 0) }
            })?;
            sockets.extend(if is_v6 { udp6(&udp) } else { udp4(&udp) });
        }
        let mut exes: HashMap<u32, Option<PathBuf>> = HashMap::new();
        for socket in &mut sockets {
            socket.exe = exes
                .entry(socket.pid)
                .or_insert_with(|| exe_path(socket.pid))
                .clone();
        }
        Ok(sockets)
    }

    /// Calls `fetch` until the buffer fits. The buffer is `u32`s so the rows
    /// (all 4-byte aligned) can be read in place.
    fn table(fetch: impl Fn(Option<*mut c_void>, *mut u32) -> u32) -> Result<Vec<u32>> {
        let mut size = 0u32;
        let mut buffer: Vec<u32> = Vec::new();
        for _ in 0..ATTEMPTS {
            let ptr = if buffer.is_empty() {
                None
            } else {
                Some(buffer.as_mut_ptr().cast::<c_void>())
            };
            let code = fetch(ptr, &mut size);
            if code == NO_ERROR.0 {
                return Ok(buffer);
            }
            if code != ERROR_INSUFFICIENT_BUFFER.0 {
                return Err(CoreError::Network(format!(
                    "the socket table could not be read (error {code})"
                )));
            }
            // Slack for sockets opened between the size query and the read.
            buffer = vec![0u32; (size as usize).div_ceil(4) + 64];
            size = u32::try_from(buffer.len() * 4).unwrap_or(u32::MAX);
        }
        Err(CoreError::Network(
            "the socket table kept changing while it was read".to_owned(),
        ))
    }

    /// Rows of type `R` behind the `dwNumEntries` header word.
    fn rows<R: Copy>(buffer: &[u32]) -> Vec<R> {
        let Some((&count, rest)) = buffer.split_first() else {
            return Vec::new();
        };
        let fit = rest.len() * 4 / std::mem::size_of::<R>();
        let base = rest.as_ptr().cast::<R>();
        // SAFETY: `count` is clamped to the rows that fit inside the buffer.
        (0..(count as usize).min(fit))
            .map(|i| unsafe { base.add(i).read_unaligned() })
            .collect()
    }

    fn v4(addr: u32) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(addr.to_ne_bytes()))
    }

    fn v6(addr: [u8; 16]) -> IpAddr {
        IpAddr::V6(Ipv6Addr::from(addr))
    }

    fn tcp4(buffer: &[u32]) -> Vec<RawSocket> {
        rows::<MIB_TCPROW_OWNER_PID>(buffer)
            .into_iter()
            .map(|r| RawSocket {
                protocol: Protocol::Tcp,
                local: v4(r.dwLocalAddr),
                local_port: port_from_net(r.dwLocalPort),
                remote: Some(v4(r.dwRemoteAddr)),
                remote_port: port_from_net(r.dwRemotePort),
                state: TcpState::from_mib(r.dwState),
                pid: r.dwOwningPid,
                exe: None,
            })
            .collect()
    }

    fn tcp6(buffer: &[u32]) -> Vec<RawSocket> {
        rows::<MIB_TCP6ROW_OWNER_PID>(buffer)
            .into_iter()
            .map(|r| RawSocket {
                protocol: Protocol::Tcp,
                local: v6(r.ucLocalAddr),
                local_port: port_from_net(r.dwLocalPort),
                remote: Some(v6(r.ucRemoteAddr)),
                remote_port: port_from_net(r.dwRemotePort),
                state: TcpState::from_mib(r.dwState),
                pid: r.dwOwningPid,
                exe: None,
            })
            .collect()
    }

    fn udp4(buffer: &[u32]) -> Vec<RawSocket> {
        rows::<MIB_UDPROW_OWNER_PID>(buffer)
            .into_iter()
            .map(|r| RawSocket {
                protocol: Protocol::Udp,
                local: v4(r.dwLocalAddr),
                local_port: port_from_net(r.dwLocalPort),
                remote: None,
                remote_port: 0,
                state: None,
                pid: r.dwOwningPid,
                exe: None,
            })
            .collect()
    }

    fn udp6(buffer: &[u32]) -> Vec<RawSocket> {
        rows::<MIB_UDP6ROW_OWNER_PID>(buffer)
            .into_iter()
            .map(|r| RawSocket {
                protocol: Protocol::Udp,
                local: v6(r.ucLocalAddr),
                local_port: port_from_net(r.dwLocalPort),
                remote: None,
                remote_port: 0,
                state: None,
                pid: r.dwOwningPid,
                exe: None,
            })
            .collect()
    }

    fn exe_path(pid: u32) -> Option<PathBuf> {
        // PID 0 (idle) and 4 (System) have no image to open.
        if pid <= 4 {
            return None;
        }
        // SAFETY: plain call; the handle is closed below.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buffer = vec![0u16; MAX_IMAGE_PATH];
        let mut len = u32::try_from(buffer.len()).ok()?;
        // SAFETY: `len` is the buffer length; the buffer outlives the call.
        let result = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut len,
            )
        };
        // SAFETY: the handle came from the successful OpenProcess above.
        let _ = unsafe { CloseHandle(handle) };
        result.ok()?;
        let end = (len as usize).min(buffer.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_is_byte_swapped_from_the_low_word() {
        // 443 = 0x01BB, stored big-endian in the low 16 bits.
        assert_eq!(port_from_net(0xBB01), 443);
        // The upper word is padding and must be ignored.
        assert_eq!(port_from_net(0xDEAD_5000), 80);
    }

    #[test]
    fn tcp_states_map_and_unknown_is_none() {
        assert_eq!(TcpState::from_mib(2), Some(TcpState::Listen));
        assert_eq!(TcpState::from_mib(5), Some(TcpState::Established));
        assert_eq!(TcpState::from_mib(0), None);
        assert_eq!(TcpState::from_mib(13), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_real_tables_can_be_read() {
        let sockets = read_sockets().expect("socket tables");
        assert!(sockets
            .iter()
            .all(|s| s.protocol == Protocol::Udp || s.remote.is_some()));
    }
}
