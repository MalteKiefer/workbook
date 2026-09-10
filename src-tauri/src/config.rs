use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::plugin::abm::AbmConnectionMeta;
use crate::plugin::action1::{Action1ConnectionMeta, Action1OrgMapping};
use crate::plugin::atera::{AteraConnectionMeta, AteraCustomerMapping};
use crate::plugin::dattormm::{DattoRmmConnectionMeta, DattoRmmSiteMapping};
use crate::plugin::intune::IntuneConnectionMeta;
use crate::plugin::iru::IruConnectionMeta;
use crate::plugin::jamf::{JamfConnectionMeta, JamfSiteMapping};
use crate::plugin::kaseya::{KaseyaConnectionMeta, KaseyaOrgMapping};
use crate::plugin::level::LevelConnectionMeta;
use crate::plugin::ninja::{NinjaConnectionMeta, NinjaOrgMapping};
use crate::plugin::pulseway::{PulsewayConnectionMeta, PulsewayOrgMapping};
use crate::plugin::snipeit::{SnipeitCompanyMapping, SnipeitConnectionMeta};
use crate::plugin::tacticalrmm::{TacticalRmmClientMapping, TacticalRmmConnectionMeta};

/// Theme preference for the UI (see `src/styles/theme.css` and
/// `src/lib/theme.ts` on the frontend side). Pure TOML/JSON serde -- no DB
/// column -- so plain serde derives are enough; `rename_all = "snake_case"`
/// makes the values appear in `config.toml` and via the Tauri command as
/// `"light"`/`"dark"`/`"system"` instead of Rust's default
/// `Light`/`Dark`/`System` spelling (same convention as
/// `db::entries::Category`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    Light,
    // The app has so far been exclusively dark -- a config.toml that gets
    // this field for the first time (existing user, old backup) must NOT
    // change its appearance because of that. Only an explicit future
    // choice may change the look.
    #[default]
    Dark,
    System,
}

/// Frequency of automatic backups (Settings → Backup). Pure TOML/serde
/// enum, same convention as `ThemePreference`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoBackupFrequency {
    #[default]
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub quick_capture: String,
    pub search: String,
    pub clipboard_screenshot: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            quick_capture: "Ctrl+Alt+Space".to_string(),
            search: "Ctrl+Alt+F".to_string(),
            clipboard_screenshot: "Ctrl+Alt+S".to_string(),
        }
    }
}

/// In-app keyboard shortcuts (Settings -> Tastaturbelegung), as opposed to
/// `HotkeyConfig`'s 3 OS-registered global hotkeys. Checked by JS keydown
/// handlers (see `src/lib/keymap.ts::matchesBinding`), not the OS, so a
/// change here takes effect immediately -- no restart, unlike `HotkeyConfig`.
///
/// `goto_customers`/`goto_systems`/`goto_journal` are two-key sequences
/// ("prefix key" then "follow-up key", space-separated, e.g. `"g c"`) since
/// that is the actual mechanism (`useGlobalHotkeys.ts`'s 800ms
/// pending-prefix state machine), not three independent key combinations --
/// `commands::keymap::set_keymap` validates all three share the same
/// prefix word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    pub command_palette: String,
    pub quick_capture: String,
    pub save: String,
    pub goto_customers: String,
    pub goto_systems: String,
    pub goto_journal: String,
    pub list_next: String,
    pub list_prev: String,
    pub edit_selected: String,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        // macOS's shipped default is the literal "Cmd+..." string, not
        // "Ctrl+..." matched loosely against both Ctrl and Cmd at runtime --
        // matching is exact (see src/lib/keymap.ts::matchesBinding), so the
        // platform-correct literal here is what makes Cmd+K work out of the
        // box on a Mac.
        let primary = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        Self {
            command_palette: format!("{primary}+K"),
            quick_capture: format!("{primary}+N"),
            save: format!("{primary}+S"),
            goto_customers: "g c".to_string(),
            goto_systems: "g s".to_string(),
            goto_journal: "g j".to_string(),
            list_next: "j".to_string(),
            list_prev: "k".to_string(),
            edit_selected: "e".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub data_dir: PathBuf,
    pub autostart_enabled: bool,
    pub context_capture_enabled: bool,
    pub late_entry_threshold_hours: i64,
    pub hotkeys: HotkeyConfig,
    pub keymap: KeymapConfig,
    pub last_customer_id: Option<i64>,
    pub last_system_id: Option<i64>,
    /// Non-secret metadata per configured Ninja connection (one set of
    /// OAuth2 credentials for exactly one Ninja tenant; a user can create as
    /// many connections as they like). A connection is NOT bound to exactly
    /// one local customer -- see `ninja_org_mappings`. The associated
    /// client ID/secret live exclusively in the OS keyring, see
    /// `plugin::secrets`.
    pub ninja_connections: Vec<NinjaConnectionMeta>,
    /// Mapping of individual Ninja "organizations" (within a connection) to
    /// local customers. A single Ninja tenant (one connection) can see
    /// multiple organizations -- e.g. because the user is themselves an MSP
    /// who runs several of their own customers as separate organizations in
    /// Ninja -- hence this separate, granular mapping table instead of a
    /// `customer_id` field directly on the connection. `#[serde(default)]`-
    /// compatible with configs from before this change that don't know this
    /// field yet (see `ninja_connections` above for the same pattern).
    pub ninja_org_mappings: Vec<NinjaOrgMapping>,
    /// Non-secret metadata per configured Level.io connection. Unlike a
    /// Ninja connection, a Level connection is bound directly to exactly one
    /// local customer (`LevelConnectionMeta.customer_id`) -- Level has no
    /// concept of organizations, see `plugin::level`. The associated API key
    /// lives exclusively in the OS keyring, see `plugin::secrets`.
    /// `#[serde(default)]`-compatible with configs from before this change,
    /// analogous to `ninja_connections` above.
    pub level_connections: Vec<LevelConnectionMeta>,
    /// Non-secret metadata per configured Snipe-IT connection (a
    /// self-hosted Snipe-IT instance; a user can create as many connections
    /// as they like). Like a Ninja connection, a Snipe-IT connection is NOT
    /// bound to exactly one local customer -- see
    /// `snipeit_company_mappings`. The associated personal access token
    /// lives exclusively in the OS keyring, see `plugin::secrets`.
    /// `#[serde(default)]`-compatible with configs from before this change,
    /// analogous to `ninja_connections` above.
    pub snipeit_connections: Vec<SnipeitConnectionMeta>,
    /// Mapping of individual Snipe-IT "companies" (within a connection) to
    /// local customers. A single Snipe-IT instance (one connection) can
    /// manage multiple companies -- e.g. because the user is themselves an
    /// MSP who runs several of their own customers as separate companies in
    /// a shared Snipe-IT instance -- hence this separate, granular mapping
    /// table instead of a `customer_id` field directly on the connection --
    /// exactly the same principle as `ninja_org_mappings`.
    /// `#[serde(default)]`-compatible with configs from before this change.
    pub snipeit_company_mappings: Vec<SnipeitCompanyMapping>,
    /// Non-secret metadata per configured Microsoft Intune connection. Like
    /// a Level connection, an Intune connection is bound directly to
    /// exactly one local customer (`IntuneConnectionMeta.customer_id`) --
    /// one Azure AD/Entra ID tenant is one organization here, no further
    /// sub-tenant concept needed, see `plugin::intune`. The associated
    /// OAuth2 credentials (tenant ID, client ID, client secret) live
    /// exclusively in the OS keyring, see `plugin::secrets`.
    /// `#[serde(default)]`-compatible with configs from before this change,
    /// analogous to `level_connections` above.
    pub intune_connections: Vec<IntuneConnectionMeta>,
    /// Non-secret metadata per configured Iru connection. Like a Level
    /// connection, an Iru connection is bound directly to exactly one local
    /// customer (`IruConnectionMeta.customer_id`) -- Iru has no concept of
    /// organizations, see `plugin::iru`. Unlike a Level connection but like a
    /// Snipe-IT connection, an Iru connection also carries a user-supplied
    /// `base_url` -- Iru is subdomain-per-tenant, not a fixed host. The
    /// associated bearer token lives exclusively in the OS keyring, see
    /// `plugin::secrets`. `#[serde(default)]`-compatible with configs from
    /// before this change, analogous to `level_connections` above.
    pub iru_connections: Vec<IruConnectionMeta>,
    /// Non-secret metadata per configured Jamf Pro connection (a self-hosted
    /// or cloud-hosted Jamf Pro server; a user can create as many
    /// connections as they like). Like a Ninja/Snipe-IT connection, a Jamf
    /// connection is NOT bound to exactly one local customer -- see
    /// `jamf_site_mappings`. The associated OAuth2 client ID/secret live
    /// exclusively in the OS keyring, see `plugin::secrets`.
    /// `#[serde(default)]`-compatible with configs from before this change,
    /// analogous to `ninja_connections` above.
    pub jamf_connections: Vec<JamfConnectionMeta>,
    /// Mapping of individual Jamf "Sites" (within a connection) to local
    /// customers. A single Jamf Pro server (one connection) can delegate
    /// inventory across multiple sites -- e.g. an MSP or a multi-campus
    /// organization -- hence this separate, granular mapping table instead
    /// of a `customer_id` field directly on the connection -- exactly the
    /// same principle as `ninja_org_mappings`/`snipeit_company_mappings`.
    /// `#[serde(default)]`-compatible with configs from before this change.
    pub jamf_site_mappings: Vec<JamfSiteMapping>,
    /// Non-secret metadata per configured Apple Business Manager (ABM)
    /// connection. Like a Level connection, an ABM connection is bound
    /// directly to exactly one local customer
    /// (`AbmConnectionMeta.customer_id`) -- ABM is inherently
    /// single-organization-scoped, no sub-tenant/site concept, see
    /// `plugin::abm`. The associated client ID/key ID/private key live
    /// exclusively in the OS keyring, see `plugin::secrets`.
    /// `#[serde(default)]`-compatible with configs from before this change,
    /// analogous to `level_connections` above.
    pub abm_connections: Vec<AbmConnectionMeta>,
    /// Non-secret metadata per configured Tactical RMM connection (a
    /// self-hosted Tactical RMM instance; a user can create as many
    /// connections as they like). Like a Ninja/Snipe-IT connection, a
    /// Tactical RMM connection is NOT bound to exactly one local customer --
    /// see `tacticalrmm_client_mappings`. The associated API key lives
    /// exclusively in the OS keyring, see `plugin::secrets`.
    /// `#[serde(default)]`-compatible with configs from before this change,
    /// analogous to `ninja_connections` above.
    pub tacticalrmm_connections: Vec<TacticalRmmConnectionMeta>,
    /// Mapping of individual Tactical RMM "Clients" (within a connection) to
    /// local customers. A single Tactical RMM instance (one connection) can
    /// manage multiple clients -- e.g. because the user is themselves an MSP
    /// who runs several of their own customers as separate clients in a
    /// shared Tactical RMM instance -- hence this separate, granular mapping
    /// table instead of a `customer_id` field directly on the connection --
    /// exactly the same principle as `ninja_org_mappings`/
    /// `snipeit_company_mappings`. `#[serde(default)]`-compatible with
    /// configs from before this change.
    pub tacticalrmm_client_mappings: Vec<TacticalRmmClientMapping>,
    /// Non-secret metadata per configured Atera connection (a cloud-hosted
    /// Atera account; a user can create as many connections as they like).
    /// Like a Ninja/Snipe-IT/Tactical-RMM connection, an Atera connection is
    /// NOT bound to exactly one local customer -- see
    /// `atera_customer_mappings`. Deliberately WITHOUT a `base_url` field --
    /// unlike Tactical RMM/NinjaOne/Snipe-IT, Atera is a single, fixed SaaS
    /// host (`plugin::atera::BASE_URL`), so there's nothing for the user to
    /// supply beyond the API key. The associated API key lives exclusively
    /// in the OS keyring, see `plugin::secrets`. `#[serde(default)]`-
    /// compatible with configs from before this change, analogous to
    /// `ninja_connections` above.
    pub atera_connections: Vec<AteraConnectionMeta>,
    /// Mapping of individual Atera "Customers" (within a connection) to
    /// local customers. A single Atera account (one connection) can manage
    /// multiple Customers -- e.g. because the user is themselves an MSP who
    /// runs several of their own customers as separate Customers in a
    /// shared Atera account -- hence this separate, granular mapping table
    /// instead of a `local_customer_id` field directly on the connection --
    /// exactly the same principle as `tacticalrmm_client_mappings`/
    /// `ninja_org_mappings`. `#[serde(default)]`-compatible with configs
    /// from before this change.
    pub atera_customer_mappings: Vec<AteraCustomerMapping>,
    /// Non-secret metadata per configured Pulseway connection (a
    /// cloud-hosted Pulseway account, or a self-hosted "Enterprise Server"
    /// instance under the same API shape; a user can create as many
    /// connections as they like). Like a Tactical RMM/Ninja/Snipe-IT
    /// connection, a Pulseway connection is NOT bound to exactly one local
    /// customer -- see `pulseway_org_mappings`. The associated Token ID/
    /// Token Secret pair lives exclusively in the OS keyring, see
    /// `plugin::secrets`. `#[serde(default)]`-compatible with configs from
    /// before this change, analogous to `tacticalrmm_connections` above.
    pub pulseway_connections: Vec<PulsewayConnectionMeta>,
    /// Mapping of individual Pulseway "Organizations" (within a connection)
    /// to local customers. A single Pulseway connection can see multiple
    /// organizations -- e.g. because the user is themselves an MSP who runs
    /// several of their own customers as separate organizations in Pulseway
    /// -- hence this separate, granular mapping table instead of a
    /// `customer_id` field directly on the connection -- exactly the same
    /// principle as `tacticalrmm_client_mappings`/`ninja_org_mappings`.
    /// `#[serde(default)]`-compatible with configs from before this change.
    pub pulseway_org_mappings: Vec<PulsewayOrgMapping>,
    /// Non-secret metadata per configured Kaseya VSA connection (a self-
    /// hosted/single-tenant VSA server; a user can create as many
    /// connections as they like). Like a Ninja/Snipe-IT/Tactical RMM
    /// connection, a Kaseya connection is NOT bound to exactly one local
    /// customer -- see `kaseya_org_mappings`. The associated Token ID/
    /// Secret pair lives exclusively in the OS keyring, see
    /// `plugin::secrets`. `#[serde(default)]`-compatible with configs from
    /// before this change, analogous to `tacticalrmm_connections` above.
    pub kaseya_connections: Vec<KaseyaConnectionMeta>,
    /// Mapping of individual Kaseya VSA "Organizations" (within a
    /// connection) to local customers. A single VSA instance (one
    /// connection) can manage multiple organizations -- e.g. because the
    /// user is themselves an MSP who runs several of their own customers as
    /// separate organizations in a shared VSA instance -- hence this
    /// separate, granular mapping table instead of a `customer_id` field
    /// directly on the connection -- exactly the same principle as
    /// `tacticalrmm_client_mappings`/`ninja_org_mappings`.
    /// `#[serde(default)]`-compatible with configs from before this change.
    pub kaseya_org_mappings: Vec<KaseyaOrgMapping>,
    /// Non-secret metadata per configured Action1 connection (a cloud-hosted
    /// Action1 account; a user can create as many connections as they
    /// like -- e.g. one per region, see `plugin::action1`). Like a Ninja/
    /// Tactical RMM connection, an Action1 connection is NOT bound to
    /// exactly one local customer -- see `action1_org_mappings`. The
    /// associated client ID/secret pair lives exclusively in the OS
    /// keyring, see `plugin::secrets`. `#[serde(default)]`-compatible with
    /// configs from before this change, analogous to `ninja_connections`
    /// above.
    pub action1_connections: Vec<Action1ConnectionMeta>,
    /// Mapping of individual Action1 "Organizations" (within a connection)
    /// to local customers. A single Action1 account (one connection) can
    /// manage multiple organizations -- e.g. because the user is themselves
    /// an MSP who runs several of their own customers as separate
    /// organizations in a shared Action1 account -- hence this separate,
    /// granular mapping table instead of a `customer_id` field directly on
    /// the connection -- exactly the same principle as `ninja_org_mappings`/
    /// `tacticalrmm_client_mappings`. `#[serde(default)]`-compatible with
    /// configs from before this change.
    pub action1_org_mappings: Vec<Action1OrgMapping>,
    /// Non-secret metadata per configured Datto RMM connection (a
    /// pod-specific Datto RMM account; a user can create as many connections
    /// as they like). Like a Ninja/Snipe-IT/Tactical-RMM/Jamf connection, a
    /// Datto RMM connection is NOT bound to exactly one local customer --
    /// see `dattormm_site_mappings`. The associated API Key/API Secret Key
    /// pair lives exclusively in the OS keyring, see `plugin::secrets`.
    /// `#[serde(default)]`-compatible with configs from before this change,
    /// analogous to `tacticalrmm_connections` above.
    pub dattormm_connections: Vec<DattoRmmConnectionMeta>,
    /// Mapping of individual Datto RMM "Sites" (within a connection) to
    /// local customers. A single Datto RMM account (one connection) can see
    /// multiple sites -- e.g. because the user is themselves an MSP who runs
    /// several of their own customers as separate sites in a shared Datto
    /// RMM account -- hence this separate, granular mapping table instead of
    /// a `customer_id` field directly on the connection -- exactly the same
    /// principle as `tacticalrmm_client_mappings`/`jamf_site_mappings`.
    /// `#[serde(default)]`-compatible with configs from before this change.
    pub dattormm_site_mappings: Vec<DattoRmmSiteMapping>,
    /// User-selected theme preference (Settings → General).
    /// `#[serde(default)]`-compatible with configs from before this field
    /// was introduced, analogous to `ninja_connections` above -- if missing,
    /// `ThemePreference::default()` (= `Dark`) applies, NOT `System`, so
    /// existing installations don't change appearance.
    pub theme_preference: ThemePreference,
    /// Whether the background scheduler (see
    /// `backup::schedule_auto_backups`) should create backups automatically.
    /// `auto_backup_dir` must also be set, otherwise the feature stays
    /// inactive despite `true` (see the `backup::is_auto_backup_due` call
    /// site in the scheduler).
    pub auto_backup_enabled: bool,
    /// Target folder for automatic backups. Separate from the manual
    /// "Create backup" dialog, which explicitly asks for the target path
    /// every time.
    pub auto_backup_dir: Option<PathBuf>,
    pub auto_backup_frequency: AutoBackupFrequency,
    /// RFC3339 timestamp (UTC) of the last successful automatic backup.
    /// `None` means "never" -- the scheduler treats that like an
    /// immediately due first run.
    pub auto_backup_last_run_utc: Option<String>,
    /// Whether both manually created and automatic backups are encrypted
    /// with the password stored in the OS keyring (see `backup::crypto`).
    /// The password itself is never stored here in `config.toml`, only this
    /// flag.
    pub backup_encryption_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            autostart_enabled: true,
            context_capture_enabled: false,
            late_entry_threshold_hours: 24,
            hotkeys: HotkeyConfig::default(),
            keymap: KeymapConfig::default(),
            last_customer_id: None,
            last_system_id: None,
            ninja_connections: Vec::new(),
            ninja_org_mappings: Vec::new(),
            level_connections: Vec::new(),
            snipeit_connections: Vec::new(),
            snipeit_company_mappings: Vec::new(),
            intune_connections: Vec::new(),
            iru_connections: Vec::new(),
            jamf_connections: Vec::new(),
            jamf_site_mappings: Vec::new(),
            abm_connections: Vec::new(),
            tacticalrmm_connections: Vec::new(),
            tacticalrmm_client_mappings: Vec::new(),
            atera_connections: Vec::new(),
            atera_customer_mappings: Vec::new(),
            pulseway_connections: Vec::new(),
            pulseway_org_mappings: Vec::new(),
            kaseya_connections: Vec::new(),
            kaseya_org_mappings: Vec::new(),
            action1_connections: Vec::new(),
            action1_org_mappings: Vec::new(),
            dattormm_connections: Vec::new(),
            dattormm_site_mappings: Vec::new(),
            theme_preference: ThemePreference::default(),
            auto_backup_enabled: false,
            auto_backup_dir: None,
            auto_backup_frequency: AutoBackupFrequency::default(),
            auto_backup_last_run_utc: None,
            backup_encryption_enabled: false,
        }
    }
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wartungsdoku")
}

pub fn resolve_data_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("WARTUNGSDOKU_DATA_DIR") {
        return PathBuf::from(override_dir);
    }
    default_data_dir()
}

impl Config {
    pub fn load_or_default(config_path: &Path) -> Result<Self, AppError> {
        if !config_path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(config_path)
            .map_err(|e| AppError::Config(format!("config.toml lesen fehlgeschlagen: {e}")))?;
        toml::from_str(&text).map_err(|e| AppError::Config(format!("config.toml ungültig: {e}")))
    }

    pub fn save(&self, config_path: &Path) -> Result<(), AppError> {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| {
            AppError::Config(format!("config.toml serialisieren fehlgeschlagen: {e}"))
        })?;
        std::fs::write(config_path, text)?;
        Ok(())
    }
}

#[cfg(test)]
// Tests build fixtures by mutating a couple of fields on a `Config::default()`
// binding; that reads clearer here than a full struct literal with
// `..Default::default()` and would only get more brittle as fields are added.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_or_default_returns_defaults_when_file_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config::load_or_default(&path).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.hotkeys.quick_capture, "Ctrl+Alt+Space");
        assert!(config.autostart_enabled);
        assert!(!config.context_capture_enabled);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let mut config = Config::default();
        config.autostart_enabled = false;
        config.context_capture_enabled = true;
        config.hotkeys.search = "Ctrl+Shift+F".to_string();

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn invalid_toml_returns_config_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not = [valid toml").unwrap();

        let result = Config::load_or_default(&path);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn save_then_load_roundtrips_last_selection() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.last_customer_id = Some(7);
        config.last_system_id = Some(3);

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.last_customer_id, Some(7));
        assert_eq!(loaded.last_system_id, Some(3));
    }

    #[test]
    fn save_then_load_roundtrips_ninja_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.ninja_connections.push(NinjaConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Ninja".to_string(),
            base_url: "https://eu.ninjarmm.com".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.ninja_connections.len(), 1);
        assert_eq!(
            loaded.ninja_connections[0].base_url,
            "https://eu.ninjarmm.com"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_ninja_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Ninja integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.ninja_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_ninja_org_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.ninja_connections.push(NinjaConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Ninja".to_string(),
            base_url: "https://eu.ninjarmm.com".to_string(),
        });
        config.ninja_org_mappings.push(NinjaOrgMapping {
            connection_id: "acme-1700000000000".to_string(),
            organization_id: "1".to_string(),
            organization_name: "ACME Hauptsitz".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.ninja_org_mappings.len(), 1);
        assert_eq!(loaded.ninja_org_mappings[0].organization_id, "1");
        assert_eq!(loaded.ninja_org_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_ninja_org_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the organization mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.ninja_org_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_level_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.level_connections.push(LevelConnectionMeta {
            id: "acme-1700000000000".to_string(),
            customer_id: 7,
            label: "ACME Level".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.level_connections.len(), 1);
        assert_eq!(loaded.level_connections[0].customer_id, 7);
        assert_eq!(loaded.level_connections[0].label, "ACME Level");
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_level_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Level.io integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.level_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_snipeit_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.snipeit_connections.len(), 1);
        assert_eq!(
            loaded.snipeit_connections[0].base_url,
            "https://assets.example.com"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_snipeit_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Snipe-IT integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.snipeit_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_snipeit_company_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });
        config.snipeit_company_mappings.push(SnipeitCompanyMapping {
            connection_id: "acme-1700000000000".to_string(),
            company_id: "1".to_string(),
            company_name: "ACME Hauptsitz".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.snipeit_company_mappings.len(), 1);
        assert_eq!(loaded.snipeit_company_mappings[0].company_id, "1");
        assert_eq!(loaded.snipeit_company_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_snipeit_company_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the company mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.snipeit_company_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_intune_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.intune_connections.push(IntuneConnectionMeta {
            id: "acme-1700000000000".to_string(),
            customer_id: 7,
            label: "ACME Intune".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.intune_connections.len(), 1);
        assert_eq!(loaded.intune_connections[0].customer_id, 7);
        assert_eq!(loaded.intune_connections[0].label, "ACME Intune");
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_intune_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Intune integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.intune_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_iru_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.iru_connections.push(IruConnectionMeta {
            id: "acme-1700000000000".to_string(),
            customer_id: 7,
            label: "ACME Iru".to_string(),
            base_url: "https://acme.api.kandji.io".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.iru_connections.len(), 1);
        assert_eq!(loaded.iru_connections[0].customer_id, 7);
        assert_eq!(loaded.iru_connections[0].label, "ACME Iru");
        assert_eq!(
            loaded.iru_connections[0].base_url,
            "https://acme.api.kandji.io"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_iru_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Iru integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.iru_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_jamf_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.jamf_connections.push(JamfConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Jamf".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.jamf_connections.len(), 1);
        assert_eq!(
            loaded.jamf_connections[0].base_url,
            "https://acme.jamfcloud.com"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_jamf_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Jamf Pro integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.jamf_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_jamf_site_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.jamf_connections.push(JamfConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Jamf".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });
        config.jamf_site_mappings.push(JamfSiteMapping {
            connection_id: "acme-1700000000000".to_string(),
            site_id: "1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.jamf_site_mappings.len(), 1);
        assert_eq!(loaded.jamf_site_mappings[0].site_id, "1");
        assert_eq!(loaded.jamf_site_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_jamf_site_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the site mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.jamf_site_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_abm_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.abm_connections.push(AbmConnectionMeta {
            id: "acme-1700000000000".to_string(),
            customer_id: 7,
            label: "ACME ABM".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.abm_connections.len(), 1);
        assert_eq!(loaded.abm_connections[0].customer_id, 7);
        assert_eq!(loaded.abm_connections[0].label, "ACME ABM");
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_abm_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the ABM integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.abm_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_tacticalrmm_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config
            .tacticalrmm_connections
            .push(TacticalRmmConnectionMeta {
                id: "acme-1700000000000".to_string(),
                label: "ACME Tactical RMM".to_string(),
                base_url: "https://api.rmm.example.com".to_string(),
            });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.tacticalrmm_connections.len(), 1);
        assert_eq!(
            loaded.tacticalrmm_connections[0].base_url,
            "https://api.rmm.example.com"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_tacticalrmm_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Tactical RMM integration
        // was introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.tacticalrmm_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_tacticalrmm_client_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config
            .tacticalrmm_connections
            .push(TacticalRmmConnectionMeta {
                id: "acme-1700000000000".to_string(),
                label: "ACME Tactical RMM".to_string(),
                base_url: "https://api.rmm.example.com".to_string(),
            });
        config
            .tacticalrmm_client_mappings
            .push(TacticalRmmClientMapping {
                connection_id: "acme-1700000000000".to_string(),
                client_id: "1".to_string(),
                client_name: "ACME Hauptsitz".to_string(),
                customer_id: 7,
            });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.tacticalrmm_client_mappings.len(), 1);
        assert_eq!(loaded.tacticalrmm_client_mappings[0].client_id, "1");
        assert_eq!(loaded.tacticalrmm_client_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_tacticalrmm_client_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the client mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.tacticalrmm_client_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_atera_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.atera_connections.push(AteraConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Atera".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.atera_connections.len(), 1);
        assert_eq!(loaded.atera_connections[0].label, "ACME Atera");
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_atera_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Atera integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.atera_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_atera_customer_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.atera_connections.push(AteraConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Atera".to_string(),
        });
        config.atera_customer_mappings.push(AteraCustomerMapping {
            connection_id: "acme-1700000000000".to_string(),
            customer_id: "1".to_string(),
            customer_name: "ACME Hauptsitz".to_string(),
            local_customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.atera_customer_mappings.len(), 1);
        assert_eq!(loaded.atera_customer_mappings[0].customer_id, "1");
        assert_eq!(loaded.atera_customer_mappings[0].local_customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_atera_customer_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the customer mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.atera_customer_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_pulseway_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.pulseway_connections.push(PulsewayConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Pulseway".to_string(),
            base_url: "https://api.pulseway.com/v3".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.pulseway_connections.len(), 1);
        assert_eq!(
            loaded.pulseway_connections[0].base_url,
            "https://api.pulseway.com/v3"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_pulseway_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Pulseway integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.pulseway_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_pulseway_org_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.pulseway_connections.push(PulsewayConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Pulseway".to_string(),
            base_url: "https://api.pulseway.com/v3".to_string(),
        });
        config.pulseway_org_mappings.push(PulsewayOrgMapping {
            connection_id: "acme-1700000000000".to_string(),
            organization_id: "6978".to_string(),
            organization_name: "Acme Corp".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.pulseway_org_mappings.len(), 1);
        assert_eq!(loaded.pulseway_org_mappings[0].organization_id, "6978");
        assert_eq!(loaded.pulseway_org_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_pulseway_org_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the organization mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.pulseway_org_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_kaseya_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.kaseya_connections.push(KaseyaConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Kaseya VSA".to_string(),
            base_url: "https://vsa.example.com/api".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.kaseya_connections.len(), 1);
        assert_eq!(
            loaded.kaseya_connections[0].base_url,
            "https://vsa.example.com/api"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_kaseya_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Kaseya VSA integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.kaseya_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_kaseya_org_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.kaseya_connections.push(KaseyaConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Kaseya VSA".to_string(),
            base_url: "https://vsa.example.com/api".to_string(),
        });
        config.kaseya_org_mappings.push(KaseyaOrgMapping {
            connection_id: "acme-1700000000000".to_string(),
            organization_id: "1".to_string(),
            organization_name: "ACME Hauptsitz".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.kaseya_org_mappings.len(), 1);
        assert_eq!(loaded.kaseya_org_mappings[0].organization_id, "1");
        assert_eq!(loaded.kaseya_org_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_kaseya_org_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the organization mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.kaseya_org_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_action1_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.action1_connections.push(Action1ConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Action1".to_string(),
            base_url: "https://app.eu.action1.com/api/3.0".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.action1_connections.len(), 1);
        assert_eq!(
            loaded.action1_connections[0].base_url,
            "https://app.eu.action1.com/api/3.0"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_action1_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Action1 integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.action1_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_action1_org_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.action1_connections.push(Action1ConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Action1".to_string(),
            base_url: "https://app.eu.action1.com/api/3.0".to_string(),
        });
        config.action1_org_mappings.push(Action1OrgMapping {
            connection_id: "acme-1700000000000".to_string(),
            organization_id: "11111111-1111-1111-1111-111111111111".to_string(),
            organization_name: "ACME Hauptsitz".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.action1_org_mappings.len(), 1);
        assert_eq!(
            loaded.action1_org_mappings[0].organization_id,
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(loaded.action1_org_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_action1_org_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the organization mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.action1_org_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_dattormm_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.dattormm_connections.push(DattoRmmConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Datto RMM".to_string(),
            base_url: "https://merlot-api.centrastage.net".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.dattormm_connections.len(), 1);
        assert_eq!(
            loaded.dattormm_connections[0].base_url,
            "https://merlot-api.centrastage.net"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_dattormm_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the Datto RMM integration was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.dattormm_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_dattormm_site_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.dattormm_connections.push(DattoRmmConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Datto RMM".to_string(),
            base_url: "https://merlot-api.centrastage.net".to_string(),
        });
        config.dattormm_site_mappings.push(DattoRmmSiteMapping {
            connection_id: "acme-1700000000000".to_string(),
            site_uid: "site-uid-1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.dattormm_site_mappings.len(), 1);
        assert_eq!(loaded.dattormm_site_mappings[0].site_uid, "site-uid-1");
        assert_eq!(loaded.dattormm_site_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_dattormm_site_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the site mapping was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to an empty list thanks to `#[serde(default)]` instead
        // of making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.dattormm_site_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_theme_preference() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.theme_preference = ThemePreference::Light;

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.theme_preference, ThemePreference::Light);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_theme_preference_field_defaults_to_dark() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before the theme selection was
        // introduced -- the field is entirely missing and must fall back
        // gracefully to `Dark` (NOT `System`) thanks to `#[serde(default)]`,
        // so the appearance of existing installations doesn't change without
        // being asked.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.theme_preference, ThemePreference::Dark);
    }

    #[test]
    fn theme_preference_serializes_as_lowercase_snake_case() {
        assert_eq!(
            serde_json::to_string(&ThemePreference::Light).unwrap(),
            "\"light\""
        );
        assert_eq!(
            serde_json::to_string(&ThemePreference::Dark).unwrap(),
            "\"dark\""
        );
        assert_eq!(
            serde_json::to_string(&ThemePreference::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn save_then_load_roundtrips_auto_backup_settings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.auto_backup_enabled = true;
        config.auto_backup_dir = Some(PathBuf::from("D:/Backups"));
        config.auto_backup_frequency = AutoBackupFrequency::Weekly;
        config.auto_backup_last_run_utc = Some("2026-09-01T10:00:00Z".to_string());
        config.backup_encryption_enabled = true;

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_auto_backup_fields_defaults_to_disabled() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before auto-backup was introduced --
        // must fall back gracefully to "disabled" thanks to
        // `#[serde(default)]`.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(!loaded.auto_backup_enabled);
        assert_eq!(loaded.auto_backup_dir, None);
        assert_eq!(loaded.auto_backup_frequency, AutoBackupFrequency::Daily);
        assert_eq!(loaded.auto_backup_last_run_utc, None);
        assert!(!loaded.backup_encryption_enabled);
    }

    #[test]
    fn resolve_data_dir_honours_env_override() {
        // SAFETY: tests run sequentially within this process for this one variable.
        std::env::set_var("WARTUNGSDOKU_DATA_DIR", "/tmp/wartungsdoku-test-override");
        let resolved = resolve_data_dir();
        std::env::remove_var("WARTUNGSDOKU_DATA_DIR");
        assert_eq!(resolved, PathBuf::from("/tmp/wartungsdoku-test-override"));
    }

    #[test]
    fn keymap_defaults_use_platform_appropriate_primary_modifier() {
        let config = Config::default();
        let expected_primary = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        assert_eq!(
            config.keymap.command_palette,
            format!("{expected_primary}+K")
        );
        assert_eq!(config.keymap.quick_capture, format!("{expected_primary}+N"));
        assert_eq!(config.keymap.save, format!("{expected_primary}+S"));
        assert_eq!(config.keymap.goto_customers, "g c");
        assert_eq!(config.keymap.goto_systems, "g s");
        assert_eq!(config.keymap.goto_journal, "g j");
        assert_eq!(config.keymap.list_next, "j");
        assert_eq!(config.keymap.list_prev, "k");
        assert_eq!(config.keymap.edit_selected, "e");
    }

    #[test]
    fn save_then_load_roundtrips_keymap() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.keymap.command_palette = "Ctrl+Shift+K".to_string();
        config.keymap.list_next = "n".to_string();

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.keymap.command_palette, "Ctrl+Shift+K");
        assert_eq!(loaded.keymap.list_next, "n");
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_keymap_field_defaults_to_platform_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before this feature existed -- the
        // field is entirely missing and must fall back gracefully to
        // KeymapConfig::default() thanks to `#[serde(default)]` instead of
        // making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.keymap, KeymapConfig::default());
    }
}
