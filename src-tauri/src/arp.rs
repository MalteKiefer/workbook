//! Reads the OS's own ARP cache to find a host's MAC address --
//! unprivileged, no raw sockets, no admin/root needed. The cache is
//! populated automatically by the kernel the moment any TCP connection is
//! attempted to a host on the same local L2 segment, which the network scan
//! (`network_scan::scan_range`) always does first -- by the time this runs,
//! every reachable local host is already in the table. A host beyond a
//! router will never have an ARP entry; that's expected, not a failure.

use std::collections::HashMap;
use std::process::Command;

use regex::Regex;

/// Reads the whole ARP cache once via `arp -a` and returns it as an
/// ip -> mac map. `arp -a`'s exact column layout differs across Windows,
/// Linux, and macOS (Windows groups entries under per-interface headers;
/// Linux/macOS print one flat list), but a generic "find a MAC-shaped token
/// on any line that also contains this IP" parse handles all three without
/// OS-specific branches -- the one wrinkle closed here is that a MAC's
/// octets may have 1 or 2 hex digits depending on platform (macOS/BSD strip
/// leading zeros, e.g. `0:1c:42:e3:b0:e1`, while Windows/Linux always print
/// two digits per octet), so the regex below accepts either width. Returns
/// an empty map (never an error) if `arp` isn't on PATH, exits non-zero, or
/// produces no parseable entries -- MAC/vendor enrichment is always
/// optional, never something a scan can fail on.
pub fn read_arp_table() -> HashMap<String, String> {
    let mut table = HashMap::new();
    let Ok(output) = Command::new("arp").arg("-a").output() else {
        return table;
    };
    if !output.status.success() {
        return table;
    }
    let mac_re = Regex::new(r"([0-9A-Fa-f]{1,2}[:-]){5}[0-9A-Fa-f]{1,2}").unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Some(mac_match) = mac_re.find(line) else {
            continue;
        };
        // The IP on an `arp -a` line is always a dotted-quad token distinct
        // from the MAC token -- split on whitespace/parens and pick the
        // first token that parses as an IPv4 address.
        let ip = line
            .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
            .find(|tok| tok.parse::<std::net::Ipv4Addr>().is_ok());
        if let Some(ip) = ip {
            table.insert(ip.to_string(), mac_match.as_str().to_string());
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_arp_table_runs_to_completion_without_panicking() {
        // No fixed-content assertion -- the real ARP cache varies by
        // machine/CI environment. This only proves the function always
        // terminates and returns a valid (possibly empty) map.
        let table = read_arp_table();
        let _ = table;
    }
}
