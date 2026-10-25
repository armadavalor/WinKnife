<!-- durum: aktif -->
# Phase 7 — network ports (C14)

Prereq: K-023. Source: `docs/modules/BACKLOG.md` (port inspector / network monitor lite), issue #15. Branch `claude/c14-network`.

## Goal
A `network` module that answers "which program is listening on this port, and who is this machine talking to?" without leaving WinKnife. Read-only, no network traffic of its own. The one action is flushing the DNS cache.

## Tasks
1. **Platform seam** (`core/platform/network.rs`). `read_sockets()` calls `GetExtendedTcpTable` / `GetExtendedUdpTable` (IPv4 + IPv6, `*_OWNER_PID` classes) with a size-probe retry loop, converts the network-byte-order ports, and resolves each PID to an exe path (`OpenProcess` + `QueryFullProcessImageNameW`). Non-Windows builds return an empty list.
2. **Module** (`modules/network.rs`). Keeps three row kinds: TCP `LISTEN`, bound UDP, TCP `ESTABLISHED`. Every row carries `reach` (`allInterfaces` / `localOnly` / `oneInterface`, from the bound address) and a plain-language `reason`. Sorted listeners first, then by port. Pure functions (`reach_of`, `rows_from`) so the logic is testable without the OS.
3. **Command** `network_list` (read-only, `spawn_blocking`). `CoreError::Network` for failures.
4. **UI** `src/routes/network/+page.svelte`: `ListToolbar` filter (process, address, port, PID, exe path), kind filter, refresh, per-row copy, "Why this row?" details. DNS flush reuses `hostsFlushDns()` / `hosts_flush_dns` behind a `ConfirmDialog` and reports success or the error text.
5. EN + TR keys for every string (K-022 parity test). Sidebar entry through the registry, `globe` icon.

## Tests
- port byte-order conversion; `reach_of` for `0.0.0.0`, `::`, loopback, a concrete address
- `rows_from` drops states that are not LISTEN / ESTABLISHED, orders listeners first, and gives every row a non-empty reason
- registry + UI-route tests cover the module id; `command_wiring` probes `network_list`

## Not in this phase
Killing a process, winsock / IP reset, firewall changes, any outbound call (reverse DNS, geo-IP). See DECISIONS-NEEDED.
