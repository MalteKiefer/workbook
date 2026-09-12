//! Best-effort "quick connect" shortcuts for a scanned host: open its web
//! UI in the browser, start an SSH session, launch Remote Desktop, or open
//! its network share -- all via whatever the OS already has installed.
//! Every one of these is opportunistic. If nothing is registered to handle
//! a given URL scheme, the OS shows its own "no application found"
//! behavior; this module never tries to detect that in advance, matching
//! this app's existing best-effort-external-tool philosophy (see
//! `nmap.rs`, `cloud_storage.rs`).

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::error::AppError;

/// Schemes this feature actually needs to open. Since `open_url` calls the
/// opener plugin's method directly and bypasses its own scope restriction,
/// this allowlist is the actual security boundary against a frontend-supplied
/// `file://` URL or arbitrary filesystem path being forwarded to the OS.
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "ssh", "rdp", "smb"];

/// Returns whether `url`'s scheme (the part before `://`) is one of
/// `ALLOWED_SCHEMES`. Pulled out of `open_url` so it can be unit-tested
/// without needing a real `AppHandle`.
fn scheme_is_allowed(url: &str) -> bool {
    url.split_once("://")
        .is_some_and(|(scheme, _)| ALLOWED_SCHEMES.contains(&scheme))
}

/// Opens `url` with whatever the OS has registered for its scheme. Calls
/// the opener plugin's method directly (not its own scope-restricted Tauri
/// command) -- see this feature's brief for why that's the correct choice
/// here, matching `commands::attachments::open_attachment`'s existing use
/// of `open_path` the same way.
pub fn open_url(app: &AppHandle, url: &str) -> Result<(), AppError> {
    if !scheme_is_allowed(url) {
        return Err(AppError::Validation(format!(
            "Nicht erlaubtes URL-Schema: \"{url}\""
        )));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::Io(format!("Konnte nicht geöffnet werden: {e}")))
}

/// Starts a Remote Desktop session to `ip`. Windows doesn't register an
/// `rdp://` URI handler out of the box, so relying on `open_url` here would
/// silently do nothing on this app's primary platform -- shell out to the
/// OS-bundled `mstsc.exe` directly instead. Elsewhere, fall back to
/// `rdp://` via the opener (some RDP clients, e.g. Microsoft Remote Desktop
/// on macOS, do register it -- best-effort, same as everything else here).
pub fn open_rdp(app: &AppHandle, ip: &str) -> Result<(), AppError> {
    match std::env::consts::OS {
        "windows" => std::process::Command::new("mstsc")
            .arg(format!("/v:{ip}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| AppError::Io(format!("Remotedesktop konnte nicht gestartet werden: {e}"))),
        _ => open_url(app, &format!("rdp://{ip}")),
    }
}

/// Opens a network-share browse view for `ip`. On Windows, opens the UNC
/// path directly in Explorer. Elsewhere, falls back to `smb://` via the
/// opener (Finder and most Linux file managers register it).
pub fn open_share(app: &AppHandle, ip: &str) -> Result<(), AppError> {
    match std::env::consts::OS {
        "windows" => std::process::Command::new("explorer")
            .arg(format!("\\\\{ip}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| {
                AppError::Io(format!(
                    "Netzwerkfreigabe konnte nicht geöffnet werden: {e}"
                ))
            }),
        _ => open_url(app, &format!("smb://{ip}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_is_allowed_accepts_the_five_schemes_this_feature_uses() {
        assert!(scheme_is_allowed("http://192.168.1.1"));
        assert!(scheme_is_allowed("https://192.168.1.1:8443"));
        assert!(scheme_is_allowed("ssh://192.168.1.1"));
        assert!(scheme_is_allowed("rdp://192.168.1.1"));
        assert!(scheme_is_allowed("smb://192.168.1.1"));
    }

    #[test]
    fn scheme_is_allowed_rejects_everything_else() {
        assert!(!scheme_is_allowed("file:///etc/passwd"));
        assert!(!scheme_is_allowed("javascript:alert(1)"));
        assert!(!scheme_is_allowed("not-a-url-at-all"));
    }
}
