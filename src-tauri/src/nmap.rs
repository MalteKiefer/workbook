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

/// Best-effort, platform-specific command to install nmap, shown in Settings
/// when `is_available()` is false. Purely informational text for the admin
/// to run themselves in their own terminal -- this app never shells out to a
/// package manager to install anything itself.
pub fn install_hint() -> &'static str {
    match std::env::consts::OS {
        "windows" => "winget install Insecure.Nmap",
        "macos" => "brew install nmap",
        _ => "sudo apt install nmap   (Debian/Ubuntu)\nsudo dnf install nmap   (Fedora/RHEL)",
    }
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

/// Runs `nmap -sV -T4 --max-parallelism 10 [-O] <target>`. `-O` (OS
/// fingerprinting) is opt-in via `os_detection` -- unlike `-sV`, it
/// typically needs raw-socket privileges (Administrator on Windows, root
/// elsewhere), so a non-elevated run simply gets nmap's own permission
/// error surfaced back through the existing error path below; this
/// function makes no attempt to detect or request elevation itself.
/// `-sS` (SYN scan) is deliberately never used for the same reason: it
/// also needs raw-socket/elevated privileges on every platform this app
/// targets, so `-sV`'s connect-based scan is used unconditionally instead.
/// `--max-parallelism 10` works around a real nmap bug (nmap/nmap#1764,
/// fixed upstream in 7.90): nmap <7.90 asserts
/// `htn.toclock_running == true` in Target.cc and crashes when scanning
/// ~100+ hosts at once (e.g. a whole /24) at default parallelism --
/// capping it avoids the crash on the older nmap builds this app has no
/// control over, and is harmless on 7.90+. Output is shown to the admin
/// as-is (see brief: no structured parsing in this feature).
pub fn run_scan(target: &str, os_detection: bool) -> Result<String, AppError> {
    validate_target(target)?;
    let mut args = vec!["-sV", "-T4", "--max-parallelism", "10"];
    if os_detection {
        args.push("-O");
    }
    args.push(target);
    let output = Command::new("nmap")
        .args(&args)
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

    #[test]
    fn install_hint_is_nonempty_on_every_platform() {
        assert!(!install_hint().is_empty());
    }
}
