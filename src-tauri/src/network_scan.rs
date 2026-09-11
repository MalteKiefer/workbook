//! Unprivileged, TCP-connect-based host discovery for an on-site network
//! scan (see this feature's brief for why this doesn't use ICMP/raw
//! sockets). A host is reported as "alive" if ANY of a small set of
//! common ports either accepts a connection or actively refuses one
//! (both prove something is listening at that IP -- a refusal is a real
//! response, just to a closed port); a timeout on every probed port
//! means nothing was found there and the host is omitted from results.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

use crate::error::AppError;

/// Common TCP ports worth probing for a general "is anything here, and
/// roughly what" signal -- not an exhaustive service list, just enough
/// spread (SSH, HTTP(S), SMB, RDP, a common alt-HTTP port) to catch most
/// servers, NAS boxes, printers, hypervisor hosts, and Windows machines
/// without the scan taking unreasonably long.
const COMMON_PORTS: &[u16] = &[22, 80, 443, 445, 3389, 8080];

const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);

/// Cap on hosts in a single scan -- keeps worst-case scan time and
/// thread usage bounded. A `/20` is 4096 addresses; anything broader
/// must be narrowed by the admin before scanning. This is a UX/resource
/// guard, not a security boundary.
const MAX_HOSTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HostScanResult {
    pub ip: String,
    pub open_ports: Vec<u16>,
}

/// Parses `cidr` (e.g. `"192.168.1.0/24"`) into the list of individual
/// host addresses to probe. For prefixes `/30` and shorter, the network
/// and broadcast addresses are excluded (matching how these ranges are
/// actually used -- neither is ever a real host); `/31` and `/32` are
/// returned as-is (point-to-point / single-host conventions where there
/// is no separate network/broadcast address).
pub fn parse_cidr(cidr: &str) -> Result<Vec<Ipv4Addr>, AppError> {
    let (addr_part, prefix_part) = cidr.split_once('/').ok_or_else(|| {
        AppError::Validation(format!(
            "Ungültiges Format \"{cidr}\" -- erwartet z.B. \"192.168.1.0/24\""
        ))
    })?;
    let base: Ipv4Addr = addr_part
        .parse()
        .map_err(|_| AppError::Validation(format!("Ungültige IP-Adresse: {addr_part}")))?;
    let prefix: u32 = prefix_part
        .parse()
        .map_err(|_| AppError::Validation(format!("Ungültiges Präfix: {prefix_part}")))?;
    if prefix == 0 || prefix > 32 {
        return Err(AppError::Validation(
            "Präfix muss zwischen 1 und 32 liegen.".to_string(),
        ));
    }
    if prefix < 20 {
        return Err(AppError::Validation(format!(
            "Bereich zu groß (/{prefix}) -- maximal /20 ({MAX_HOSTS} Adressen) pro Scan."
        )));
    }

    let base_u32 = u32::from(base);
    let host_bits = 32 - prefix;
    let mask = if host_bits == 32 {
        0
    } else {
        !0u32 << host_bits
    };
    let network = base_u32 & mask;
    let broadcast = network | !mask;

    let addrs: Vec<Ipv4Addr> = if prefix >= 31 {
        (network..=broadcast).map(Ipv4Addr::from).collect()
    } else {
        ((network + 1)..broadcast).map(Ipv4Addr::from).collect()
    };
    if addrs.len() > MAX_HOSTS {
        return Err(AppError::Validation(format!(
            "Bereich zu groß ({} Adressen) -- maximal {MAX_HOSTS} pro Scan.",
            addrs.len()
        )));
    }
    Ok(addrs)
}

/// Probes every port in `COMMON_PORTS` for `ip` in a single pass, tracking
/// both which ports actually opened and whether any port was actively
/// refused (as opposed to timing out, which is silent evidence of
/// nothing). The host is reported (`Some`) if either signal fired; it's
/// omitted (`None`) only when every probed port timed out, meaning
/// nothing responded at all. See the brief's note on an earlier two-pass
/// version of this function -- this single pass keeps the exact same
/// "alive" semantics without the redundant re-connect.
fn probe_host(ip: Ipv4Addr) -> Option<HostScanResult> {
    let mut open_ports = Vec::new();
    let mut saw_refusal = false;
    for &port in COMMON_PORTS {
        let addr = SocketAddr::new(IpAddr::V4(ip), port);
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(_stream) => open_ports.push(port),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => saw_refusal = true,
            Err(_) => {} // timeout or other error -- no evidence either way
        }
    }
    if open_ports.is_empty() && !saw_refusal {
        return None;
    }
    Some(HostScanResult {
        ip: ip.to_string(),
        open_ports,
    })
}

/// Scans every address in `ips` concurrently (bounded by `max_concurrency`
/// simultaneous threads, since a fully sequential scan of e.g. 254
/// addresses × 6 ports × 400ms would take minutes) and returns only the
/// hosts that responded. Uses plain `std::thread` + a channel rather than
/// pulling in a thread-pool crate -- this codebase has no async runtime
/// and no existing thread-pool dependency, and a scan is a one-shot,
/// bounded-size batch of independent work, not a recurring need that
/// would justify a persistent pool.
pub fn scan_range(ips: Vec<Ipv4Addr>, max_concurrency: usize) -> Vec<HostScanResult> {
    let (tx, rx) = mpsc::channel();
    let chunks: Vec<Vec<Ipv4Addr>> = ips
        .chunks(ips.len().div_ceil(max_concurrency.max(1)).max(1))
        .map(|c| c.to_vec())
        .collect();

    let handles: Vec<_> = chunks
        .into_iter()
        .map(|chunk| {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for ip in chunk {
                    if let Some(result) = probe_host(ip) {
                        let _ = tx.send(result);
                    }
                }
            })
        })
        .collect();
    drop(tx);

    let mut results: Vec<HostScanResult> = rx.iter().collect();
    for handle in handles {
        let _ = handle.join();
    }
    results.sort_by(|a, b| a.ip.cmp(&b.ip));
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cidr_slash_24_excludes_network_and_broadcast() {
        let addrs = parse_cidr("192.168.1.0/24").unwrap();
        assert_eq!(addrs.len(), 254);
        assert_eq!(addrs[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(addrs[253], Ipv4Addr::new(192, 168, 1, 254));
    }

    #[test]
    fn parse_cidr_slash_30_excludes_network_and_broadcast() {
        let addrs = parse_cidr("10.0.0.0/30").unwrap();
        // 10.0.0.0 (network), 10.0.0.1, 10.0.0.2, 10.0.0.3 (broadcast)
        assert_eq!(
            addrs,
            vec![Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2)]
        );
    }

    #[test]
    fn parse_cidr_slash_31_includes_both_addresses() {
        let addrs = parse_cidr("10.0.0.0/31").unwrap();
        assert_eq!(
            addrs,
            vec![Ipv4Addr::new(10, 0, 0, 0), Ipv4Addr::new(10, 0, 0, 1)]
        );
    }

    #[test]
    fn parse_cidr_slash_32_includes_single_address() {
        let addrs = parse_cidr("10.0.0.5/32").unwrap();
        assert_eq!(addrs, vec![Ipv4Addr::new(10, 0, 0, 5)]);
    }

    #[test]
    fn parse_cidr_rejects_missing_prefix() {
        let err = parse_cidr("192.168.1.0").unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn parse_cidr_rejects_invalid_ip() {
        let err = parse_cidr("not-an-ip/24").unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn parse_cidr_rejects_prefix_out_of_range() {
        assert!(parse_cidr("10.0.0.0/0").is_err());
        assert!(parse_cidr("10.0.0.0/33").is_err());
    }

    #[test]
    fn parse_cidr_rejects_range_broader_than_slash_20() {
        let err = parse_cidr("10.0.0.0/8").unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn scan_range_on_localhost_finds_no_unexpected_hosts() {
        // No live-network assertions here (no guaranteed-reachable test
        // host in CI) -- this only proves scan_range runs to completion,
        // spawns/joins its threads cleanly, and returns a sorted (possibly
        // empty) Vec without hanging or panicking, using a tiny, safe,
        // guaranteed-local range.
        let addrs = vec![Ipv4Addr::new(127, 0, 0, 1)];
        let results = scan_range(addrs, 4);
        // 127.0.0.1 may or may not have anything listening in a CI
        // sandbox -- only assert it terminates and returns a valid Vec.
        let _ = results;
    }
}
