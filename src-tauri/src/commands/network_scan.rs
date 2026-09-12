use tauri::State;

use crate::{network_scan, nmap, snmp_probe, AppError, AppState};

#[tauri::command]
pub fn scan_network(
    cidr: String,
    state: State<AppState>,
) -> Result<Vec<network_scan::HostScanResult>, AppError> {
    let ports = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        config.network_scan_ports.clone()
    };
    let addrs = network_scan::parse_cidr(&cidr)?;
    Ok(network_scan::scan_range(addrs, 64, &ports))
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

/// Non-secret network-scan settings surfaced in Settings -> Netzwerk.
#[derive(Debug, serde::Serialize)]
pub struct NetworkScanSettingsDto {
    pub ports: Vec<u16>,
}

#[tauri::command]
pub fn get_network_scan_settings(state: State<AppState>) -> NetworkScanSettingsDto {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    NetworkScanSettingsDto {
        ports: config.network_scan_ports.clone(),
    }
}

#[tauri::command]
pub fn set_network_scan_settings(state: State<AppState>, ports: Vec<u16>) -> Result<(), AppError> {
    if ports.is_empty() {
        return Err(AppError::Validation(
            "Mindestens ein Port muss angegeben werden.".to_string(),
        ));
    }
    if ports.contains(&0) {
        return Err(AppError::Validation(
            "Port 0 ist kein gültiger TCP-Port.".to_string(),
        ));
    }
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.network_scan_ports = ports;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

/// Best-effort, platform-specific install command shown in Settings when
/// `is_nmap_available()` is false. Text only -- never executed by this app.
#[tauri::command]
pub fn nmap_install_hint() -> String {
    nmap::install_hint().to_string()
}
