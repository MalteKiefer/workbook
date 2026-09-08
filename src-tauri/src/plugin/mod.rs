//! Erweiterungspunkt für externe Integrationen (z. B. RMM-Tools), Spec-Abschnitt
//! "Plugin-Architektur". Jetzt nur die Isolationsgrenze selbst: Trait-Definition,
//! Hilfstypen, eine dokumentierte Dummy-Implementierung (`dummy`) und ein
//! OS-Schlüsselspeicher-Wrapper (`secrets`). Kein Plugin-Loader, kein UI-Aufruf —
//! das ist bewusst spätere Ausbaustufe.

/// Ein System/Gerät, wie es ein externes Verwaltungswerkzeug (z. B. eine
/// RMM-Plattform) meldet. Bewusst minimal und generisch -- reichhaltige,
/// werkzeugspezifische Daten gehören in `external_refs.payload_json`, nicht
/// hierher.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystem {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
}

/// Opaques Zugangsdaten-Bündel, das ein Plugin braucht, um sich gegen seinen
/// externen Dienst zu authentifizieren. Das Plugin interpretiert seine eigenen
/// Felder (z. B. API-Key vs. Benutzername+Passwort) -- dieses Crate schaut
/// nie hinein, holt die Daten nur unversehrt aus dem OS-Schlüsselspeicher
/// (siehe `secrets`) und reicht sie durch.
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

/// Isolationsgrenze für eine externe Integration (z. B. ein RMM-Tool). Ein
/// Plugin fasst nie SQLite direkt an -- der Aufrufer ist dafür zuständig, das
/// Ergebnis von `list_systems`/`get_system_details` in die Tabelle
/// `external_refs` zu schreiben (siehe `db::external_refs`), dabei immer als
/// extern markiert, nie überschreibend, was der Nutzer an einem System selbst
/// gepflegt hat.
///
/// Jede Trait-Methode ist eine reine, synchrone Funktion mit Rückgabewert --
/// keine Rückgabe erzwingt später eine maus-exklusive Bedienung (kein Callback
///, der einen Dialog voraussetzt, kein Rückgabetyp, der nur per Klick
/// aufzulösen wäre). Eine künftige Command-Palette kann jede Methode direkt
/// hinter einen Tastaturbefehl hängen.
pub trait Plugin {
    /// Eindeutiger Bezeichner des Plugins, z. B. `"dummy"`. Dient als
    /// `plugin_id` in `external_refs` und als Konto-Name im Schlüsselspeicher
    /// (siehe `secrets`).
    fn id(&self) -> &str;

    /// Listet alle Systeme, die der externe Dienst für diese Zugangsdaten
    /// kennt.
    fn list_systems(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<ExternalSystem>, PluginError>;

    /// Liefert Detaildaten zu genau einem externen System als freies JSON --
    /// die Form ist werkzeugspezifisch, deshalb kein fester Rust-Typ. Landet
    /// unverändert in `external_refs.payload_json`.
    fn get_system_details(
        &self,
        credentials: &PluginCredentials,
        external_id: &str,
    ) -> Result<serde_json::Value, PluginError>;

    /// Vermerkt, dass ein lokales System (`local_system_id`, `systems.id`)
    /// einem externen System (`external_id`) entspricht. Diese Methode selbst
    /// fasst die Datenbank nicht an -- der Aufrufer persistiert die
    /// Verknüpfung anschließend über `db::external_refs::upsert`.
    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError>;
}

pub mod dummy;
pub mod level;
pub mod ninja;
pub mod secrets;
pub mod snipeit;
