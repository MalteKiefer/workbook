use tauri::State;

use crate::{network_scan, nmap, snmp_probe, AppError, AppState};

#[tauri::command]
pub async fn scan_network(
    cidr: String,
    state: State<'_, AppState>,
) -> Result<Vec<network_scan::HostScanResult>, AppError> {
    let ports = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        config.network_scan_ports.clone()
    };
    let ports = if ports.is_empty() {
        network_scan::DEFAULT_PORTS.to_vec()
    } else {
        ports
    };

    tauri::async_runtime::spawn_blocking(
        move || -> Result<Vec<network_scan::HostScanResult>, AppError> {
            let addrs = network_scan::parse_cidr(&cidr)?;
            let results = network_scan::scan_range(addrs, 64, &ports);

            let arp_table = crate::arp::read_arp_table();
            let results = results
                .into_iter()
                .map(|mut result| {
                    result.mac = arp_table.get(&result.ip).cloned();
                    result.vendor = result
                        .mac
                        .as_deref()
                        .and_then(crate::mac_vendor::lookup_vendor);
                    result.hostname = result.ip.parse().ok().and_then(|ip| {
                        crate::reverse_dns::lookup_hostname_with_timeout(
                            ip,
                            std::time::Duration::from_millis(500),
                        )
                    });
                    result
                })
                .collect();

            Ok(results)
        },
    )
    .await
    .map_err(|e| AppError::Io(format!("Scan-Task fehlgeschlagen: {e}")))?
}

#[tauri::command]
pub async fn probe_snmp(
    ip: String,
    community: String,
) -> Result<snmp_probe::SnmpProbeResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || snmp_probe::probe(&ip, &community))
        .await
        .map_err(|e| AppError::Io(format!("SNMP-Abfrage-Task fehlgeschlagen: {e}")))?
}

#[tauri::command]
pub async fn is_nmap_available() -> bool {
    tauri::async_runtime::spawn_blocking(nmap::is_available)
        .await
        .unwrap_or(false)
}

#[tauri::command]
pub async fn run_nmap_scan(target: String) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || nmap::run_scan(&target))
        .await
        .map_err(|e| AppError::Io(format!("nmap-Task fehlgeschlagen: {e}")))?
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
    let mut ports = ports;
    ports.sort_unstable();
    ports.dedup();
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
