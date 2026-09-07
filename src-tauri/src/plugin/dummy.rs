use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Referenzimplementierung des `Plugin`-Traits, liefert feste Beispieldaten.
/// Zweck: beweisen, dass die Trait-Form tatsächlich Ende-zu-Ende benutzbar
/// ist, und einer künftigen echten Integration (z. B. einem RMM-Connector)
/// eine konkrete, funktionierende Vorlage zum Abschreiben geben -- nicht
/// dazu gedacht, als echtes Feature ausgeliefert zu werden. Kein Netzwerk-
/// zugriff, vollständig in-memory.
pub struct DummyPlugin;

impl Plugin for DummyPlugin {
    fn id(&self) -> &str {
        "dummy"
    }

    fn list_systems(&self, _credentials: &PluginCredentials) -> Result<Vec<ExternalSystem>, PluginError> {
        Ok(vec![
            ExternalSystem { external_id: "dummy-1".into(), name: "Dummy-Server".into(), hostname: Some("dummy-server.local".into()) },
            ExternalSystem { external_id: "dummy-2".into(), name: "Dummy-Firewall".into(), hostname: Some("dummy-fw.local".into()) },
        ])
    }

    fn get_system_details(&self, _credentials: &PluginCredentials, external_id: &str) -> Result<serde_json::Value, PluginError> {
        Ok(serde_json::json!({ "external_id": external_id, "status": "ok", "note": "Beispieldaten der Attrappen-Implementierung" }))
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        println!("DummyPlugin: verknüpfe lokales System {local_system_id} mit externer ID {external_id}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_systems_returns_sample_data() {
        let plugin = DummyPlugin;
        let creds = PluginCredentials { secret: "unused".into() };
        let systems = plugin.list_systems(&creds).unwrap();
        assert_eq!(systems.len(), 2);
        assert_eq!(systems[0].external_id, "dummy-1");
    }

    #[test]
    fn get_system_details_echoes_external_id() {
        let plugin = DummyPlugin;
        let creds = PluginCredentials { secret: "unused".into() };
        let details = plugin.get_system_details(&creds, "dummy-1").unwrap();
        assert_eq!(details["external_id"], "dummy-1");
    }

    #[test]
    fn link_system_succeeds() {
        let plugin = DummyPlugin;
        assert!(plugin.link_system(42, "dummy-1").is_ok());
    }
}
