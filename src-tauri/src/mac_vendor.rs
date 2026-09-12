//! Best-effort MAC-address-to-manufacturer lookup using the IEEE OUI
//! database bundled into this binary at compile time via the `mac_oui`
//! crate's `with-db` feature (~51,000 records, no network access, no
//! runtime file needed). A miss (address not in the database, or a
//! locally-administered/randomized address -- common on phones and modern
//! laptops for privacy) is a normal, silent `None`, never an error.

use std::sync::OnceLock;

use mac_oui::Oui;

fn db() -> Option<&'static Oui> {
    static DB: OnceLock<Option<Oui>> = OnceLock::new();
    DB.get_or_init(|| Oui::default().ok()).as_ref()
}

/// Returns the registered manufacturer name for `mac` (any of the common
/// colon- or hyphen-separated formats, e.g. `"dc:9f:db:12:34:56"` or
/// `"DC-9F-DB-12-34-56"`), or `None` if the address isn't in the database,
/// doesn't parse, or the database itself failed to load.
pub fn lookup_vendor(mac: &str) -> Option<String> {
    db()?
        .lookup_by_mac(mac)
        .ok()?
        .map(|entry| entry.company_name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_vendor_finds_a_known_ieee_registered_prefix() {
        // 70:B3:D5 is IEEE's own registration authority block -- present in
        // every edition of the OUI database, safe to assert on indefinitely.
        let vendor = lookup_vendor("70:B3:D5:00:00:00");
        assert!(vendor.is_some());
    }

    #[test]
    fn lookup_vendor_returns_none_for_garbage_input() {
        assert_eq!(lookup_vendor("not-a-mac-address"), None);
    }

    #[test]
    fn lookup_vendor_returns_none_for_an_unassigned_prefix() {
        // FF:FF:FF is the broadcast address, never individually assigned.
        assert_eq!(lookup_vendor("FF:FF:FF:FF:FF:FF"), None);
    }
}
