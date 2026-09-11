//! SNMPv1/v2c probe for a single host -- fetches a handful of standard
//! `system` MIB values (sysDescr, sysName, sysLocation, sysUpTime) via a
//! community string, the same read-only, no-elevated-privileges
//! mechanism most network gear (switches, UPSes, printers, APs) exposes
//! out of the box. SNMPv3 is out of scope, see this feature's brief.

use std::time::Duration;

use snmp2::{Oid, SyncSession, Value};

use crate::error::AppError;

const SNMP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnmpProbeResult {
    pub sys_descr: Option<String>,
    pub sys_name: Option<String>,
    pub sys_location: Option<String>,
    pub sys_up_time: Option<String>,
}

fn value_to_display_string(value: &Value) -> String {
    match value {
        Value::OctetString(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        other => format!("{other:?}"),
    }
}

/// Queries `ip:161` with `community`, returning whatever of the four
/// standard OIDs the device answered (a device may legitimately leave
/// some unset, e.g. no sysLocation configured -- that's `None`, not an
/// error). Only a genuine connection/timeout/auth failure (nothing
/// answered at all) is an `Err`.
pub fn probe(ip: &str, community: &str) -> Result<SnmpProbeResult, AppError> {
    let sys_descr_oid = Oid::from(&[1, 3, 6, 1, 2, 1, 1, 1, 0])
        .map_err(|e| AppError::Validation(format!("Ungültige OID: {e:?}")))?;
    let sys_up_time_oid = Oid::from(&[1, 3, 6, 1, 2, 1, 1, 3, 0])
        .map_err(|e| AppError::Validation(format!("Ungültige OID: {e:?}")))?;
    let sys_name_oid = Oid::from(&[1, 3, 6, 1, 2, 1, 1, 5, 0])
        .map_err(|e| AppError::Validation(format!("Ungültige OID: {e:?}")))?;
    let sys_location_oid = Oid::from(&[1, 3, 6, 1, 2, 1, 1, 6, 0])
        .map_err(|e| AppError::Validation(format!("Ungültige OID: {e:?}")))?;

    let agent_addr = format!("{ip}:161");
    let mut session =
        SyncSession::new_v2c(&agent_addr, community.as_bytes(), Some(SNMP_TIMEOUT), 0)
            .map_err(|e| AppError::Validation(format!("SNMP-Verbindung fehlgeschlagen: {e}")))?;

    let oids = [
        &sys_descr_oid,
        &sys_up_time_oid,
        &sys_name_oid,
        &sys_location_oid,
    ];
    let response = session
        .get_many(&oids)
        .map_err(|e| AppError::Validation(format!("Keine Antwort vom Gerät: {e:?}")))?;

    let mut result = SnmpProbeResult {
        sys_descr: None,
        sys_name: None,
        sys_location: None,
        sys_up_time: None,
    };
    for (oid, value) in response.varbinds {
        let display = value_to_display_string(&value);
        if oid == sys_descr_oid {
            result.sys_descr = Some(display);
        } else if oid == sys_up_time_oid {
            result.sys_up_time = Some(display);
        } else if oid == sys_name_oid {
            result.sys_name = Some(display);
        } else if oid == sys_location_oid {
            result.sys_location = Some(display);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_against_a_closed_local_port_fails_cleanly() {
        // No live SNMP agent available in CI/this environment -- this
        // only proves a probe against something that will NOT answer
        // (localhost, almost certainly nothing on UDP 161) returns a
        // clean Err rather than hanging indefinitely or panicking. The
        // 2-second SNMP_TIMEOUT bounds this test's runtime.
        let result = probe("127.0.0.1", "public");
        assert!(result.is_err());
    }
}
