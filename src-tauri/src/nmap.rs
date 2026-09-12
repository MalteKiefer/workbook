//! Shells out to a system-installed `nmap` binary when present -- never
//! bundled with this app (see this feature's brief for why). Uses
//! `std::process::Command` directly (never a shell), so there is no
//! shell-injection surface regardless of `target`'s contents; `target`
//! is still validated as a plausible IP/CIDR before invoking, purely so
//! a typo produces a clear error instead of a confusing nmap failure.

use std::process::Command;

use regex::Regex;

use crate::error::AppError;

/// Whether `nmap` is on PATH and runnable at all.
pub fn is_available() -> bool {
    Command::new("nmap")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn validate_target(target: &str) -> Result<(), AppError> {
    // Deliberately permissive (IPv4 address, optionally with a /prefix)
    // -- just enough to reject obviously-wrong input (empty string,
    // stray whitespace, a pasted URL) before spending time invoking an
    // external process, not a full CIDR validator (network_scan::parse_cidr
    // already does that more strictly for the TCP sweep's own input).
    let re = Regex::new(r"^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(/\d{1,2})?$").unwrap();
    if !re.is_match(target) {
        return Err(AppError::Validation(format!(
            "Ungültiges Ziel für nmap: \"{target}\" -- erwartet eine IP-Adresse oder ein CIDR wie 192.168.1.0/24"
        )));
    }
    Ok(())
}

/// Runs `nmap -sV -T4 --max-parallelism 10 <target>` (version detection
/// over a plain TCP connect scan -- deliberately NOT `-sS`/`-O`, which
/// need raw sockets and elevated privileges on every platform this app
/// targets) and returns its combined stdout, or an error if nmap isn't
/// installed or exits non-zero. `--max-parallelism 10` works around a
/// real nmap bug (nmap/nmap#1764, fixed upstream in 7.90): nmap <7.90
/// asserts `htn.toclock_running == true` in Target.cc and crashes when
/// scanning ~100+ hosts at once (e.g. a whole /24) at default
/// parallelism -- capping it avoids the crash on the older nmap builds
/// this app has no control over, and is harmless on 7.90+. Output is
/// shown to the admin as-is (see brief: no structured parsing in this
/// feature).
pub fn run_scan(target: &str) -> Result<String, AppError> {
    validate_target(target)?;
    let output = Command::new("nmap")
        .args(["-sV", "-T4", "--max-parallelism", "10", target])
        .output()
        .map_err(|e| AppError::Validation(format!("nmap konnte nicht gestartet werden: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Validation(format!(
            "nmap-Scan fehlgeschlagen: {stderr}"
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_target_accepts_plain_ip() {
        assert!(validate_target("192.168.1.1").is_ok());
    }

    #[test]
    fn validate_target_accepts_cidr() {
        assert!(validate_target("192.168.1.0/24").is_ok());
    }

    #[test]
    fn validate_target_rejects_garbage() {
        assert!(validate_target("").is_err());
        assert!(validate_target("not an ip; rm -rf /").is_err());
        assert!(validate_target("http://example.com").is_err());
    }
}
