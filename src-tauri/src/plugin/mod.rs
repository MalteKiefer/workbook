//! Extension point for external integrations (e.g. RMM tools), spec section
//! "Plugin architecture". For now just the isolation boundary itself: trait
//! definition, helper types, a documented dummy implementation (`dummy`) and
//! an OS keyring wrapper (`secrets`). No plugin loader, no UI hookup yet --
//! that is deliberately a later expansion stage.

/// A system/device as reported by an external management tool (e.g. an RMM
/// platform). Deliberately minimal and generic -- rich, tool-specific data
/// belongs in `external_refs.payload_json`, not here.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystem {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
}

/// Opaque credential bundle a plugin needs to authenticate against its
/// external service. The plugin interprets its own fields (e.g. API key vs.
/// username+password) -- this crate never looks inside, it just retrieves
/// the data unmodified from the OS keyring (see `secrets`) and passes it
/// through.
#[derive(Debug, Clone)]
pub struct PluginCredentials {
    pub secret: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("Authentifizierung fehlgeschlagen: {0}")]
    Authentication(String),
    #[error("Externer Dienst nicht erreichbar: {0}")]
    Unreachable(String),
    #[error("Unerwartete Antwort des externen Dienstes: {0}")]
    UnexpectedResponse(String),
}

/// Isolation boundary for an external integration (e.g. an RMM tool). A
/// plugin never touches SQLite directly -- the caller is responsible for
/// writing the result of `list_systems`/`get_system_details` into the
/// `external_refs` table (see `db::external_refs`), always marked as
/// external, never overwriting what the user has maintained on a system
/// themselves.
///
/// Every trait method is a pure, synchronous function with a return value --
/// no return value later forces mouse-exclusive operation (no callback that
/// requires a dialog, no return type that could only be resolved by a click).
/// A future command palette can hang any method directly behind a keyboard
/// shortcut.
pub trait Plugin {
    /// Unique identifier of the plugin, e.g. `"dummy"`. Serves as `plugin_id`
    /// in `external_refs` and as the account name in the keyring (see
    /// `secrets`).
    fn id(&self) -> &str;

    /// Lists all systems the external service knows about for these
    /// credentials.
    fn list_systems(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<ExternalSystem>, PluginError>;

    /// Returns detail data for exactly one external system as free-form JSON
    /// -- the shape is tool-specific, hence no fixed Rust type. Lands
    /// unmodified in `external_refs.payload_json`.
    fn get_system_details(
        &self,
        credentials: &PluginCredentials,
        external_id: &str,
    ) -> Result<serde_json::Value, PluginError>;

    /// Records that a local system (`local_system_id`, `systems.id`)
    /// corresponds to an external system (`external_id`). This method itself
    /// does not touch the database -- the caller persists the link
    /// afterwards via `db::external_refs::upsert`.
    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError>;
}

pub mod abm;
pub mod acronis;
pub mod dummy;
pub mod intune;
pub mod iru;
pub mod jamf;
pub mod level;
pub mod ninja;
pub mod secrets;
pub mod snipeit;
pub mod tacticalrmm;
