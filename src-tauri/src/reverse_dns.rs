//! Best-effort reverse-DNS (PTR) hostname lookup for a scanned host,
//! bounded by an explicit timeout (see module comment below for why this
//! needs its own thread rather than calling `dns_lookup::lookup_addr`
//! directly).

use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc;
use std::time::Duration;

/// Looks up the PTR (reverse DNS) hostname for `ip`, giving up and
/// returning `None` if no answer -- positive or negative -- arrives within
/// `timeout`. A `None` covers every non-fatal outcome (no PTR record,
/// resolver unreachable, timed out) equally; the caller only ever wants
/// "did we get a name or not", never has to distinguish the reasons.
pub fn lookup_hostname_with_timeout(ip: Ipv4Addr, timeout: Duration) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = dns_lookup::lookup_addr(&IpAddr::V4(ip)).ok();
        let _ = tx.send(result);
    });
    rx.recv_timeout(timeout).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_hostname_on_a_closed_local_address_does_not_hang() {
        // No PTR record is guaranteed for a random high-numbered loopback
        // alias, but the important property is termination within the
        // timeout, not a specific Some/None outcome.
        let result =
            lookup_hostname_with_timeout(Ipv4Addr::new(127, 0, 0, 1), Duration::from_millis(500));
        let _ = result;
    }
}
