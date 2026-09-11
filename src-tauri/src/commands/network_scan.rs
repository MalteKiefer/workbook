use crate::{network_scan, nmap, snmp_probe, AppError};

#[tauri::command]
pub fn scan_network(cidr: String) -> Result<Vec<network_scan::HostScanResult>, AppError> {
    let addrs = network_scan::parse_cidr(&cidr)?;
    Ok(network_scan::scan_range(addrs, 64))
}

#[tauri::command]
pub fn probe_snmp(ip: String, community: String) -> Result<snmp_probe::SnmpProbeResult, AppError> {
    snmp_probe::probe(&ip, &community)
}

#[tauri::command]
pub fn is_nmap_available() -> bool {
    nmap::is_available()
}

#[tauri::command]
pub fn run_nmap_scan(target: String) -> Result<String, AppError> {
    nmap::run_scan(&target)
}
