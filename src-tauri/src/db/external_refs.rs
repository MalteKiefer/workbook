use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExternalRef {
    pub id: i64,
    pub system_id: i64,
    pub plugin_id: String,
    pub external_id: String,
    pub payload_json: String,
    pub synced_at_utc: String,
    pub synced_at_tz: String,
}

fn row_to_external_ref(row: &Row) -> rusqlite::Result<ExternalRef> {
    Ok(ExternalRef {
        id: row.get("id")?,
        system_id: row.get("system_id")?,
        plugin_id: row.get("plugin_id")?,
        external_id: row.get("external_id")?,
        payload_json: row.get("payload_json")?,
        synced_at_utc: row.get("synced_at_utc")?,
        synced_at_tz: row.get("synced_at_tz")?,
    })
}

/// Fügt eine Verknüpfung zwischen einem lokalen System und einem externen
/// Plugin-System ein oder aktualisiert sie -- ein System hat höchstens eine
/// Zeile je Plugin (`system_id`, `plugin_id`). Erneutes Synchronisieren
/// aktualisiert `payload_json`/`synced_at_*` in derselben Zeile, statt
/// Duplikate anzusammeln.
pub fn upsert(
    conn: &Connection,
    system_id: i64,
    plugin_id: &str,
    external_id: &str,
    payload_json: &str,
    tz: &Tz,
) -> Result<ExternalRef, AppError> {
    let (synced_at_utc, synced_at_tz) = now_with_tz(tz);
    let existing_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM external_refs WHERE system_id = ?1 AND plugin_id = ?2",
            params![system_id, plugin_id],
            |r| r.get(0),
        )
        .optional()?;
    let id = match existing_id {
        Some(id) => {
            conn.execute(
                "UPDATE external_refs SET external_id = ?1, payload_json = ?2, synced_at_utc = ?3, synced_at_tz = ?4 WHERE id = ?5",
                params![external_id, payload_json, synced_at_utc, synced_at_tz, id],
            )?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO external_refs (system_id, plugin_id, external_id, payload_json, synced_at_utc, synced_at_tz) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![system_id, plugin_id, external_id, payload_json, synced_at_utc, synced_at_tz],
            )?;
            conn.last_insert_rowid()
        }
    };
    get(conn, id)
}

pub fn get(conn: &Connection, id: i64) -> Result<ExternalRef, AppError> {
    conn.query_row("SELECT * FROM external_refs WHERE id = ?1", params![id], row_to_external_ref)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("external_ref {id} nicht gefunden")))
}

pub fn list_for_system(conn: &Connection, system_id: i64) -> Result<Vec<ExternalRef>, AppError> {
    let mut stmt = conn.prepare("SELECT * FROM external_refs WHERE system_id = ?1 ORDER BY plugin_id")?;
    let rows = stmt.query_map(params![system_id], row_to_external_ref)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::customers::{self, NewCustomer};
    use crate::db::systems::{self, NewSystem};
    use crate::db::test_support::migrated_connection;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    fn seed_system(conn: &Connection) -> i64 {
        let customer_id = customers::create(conn, NewCustomer { name: "ACME".into(), short_code: "ACME".into(), notes: "".into() }, &berlin())
            .unwrap()
            .id;
        systems::create(
            conn,
            NewSystem { customer_id, name: "FS01".into(), system_type: "Server".into(), hostname: "fs01.acme.local".into(), ip_address: "10.0.0.5".into(), notes: "".into() },
            &berlin(),
        )
        .unwrap()
        .id
    }

    #[test]
    fn upsert_twice_for_same_plugin_updates_single_row() {
        let conn = migrated_connection();
        let system_id = seed_system(&conn);

        let first = upsert(&conn, system_id, "dummy", "dummy-1", r#"{"status":"ok"}"#, &berlin()).unwrap();
        let second = upsert(&conn, system_id, "dummy", "dummy-1", r#"{"status":"changed"}"#, &berlin()).unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.payload_json, r#"{"status":"changed"}"#);
        assert_eq!(list_for_system(&conn, system_id).unwrap().len(), 1);
    }

    #[test]
    fn upsert_for_different_plugin_adds_second_row() {
        let conn = migrated_connection();
        let system_id = seed_system(&conn);

        upsert(&conn, system_id, "dummy", "dummy-1", "{}", &berlin()).unwrap();
        upsert(&conn, system_id, "other-plugin", "ext-9", "{}", &berlin()).unwrap();

        let refs = list_for_system(&conn, system_id).unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].plugin_id, "dummy");
        assert_eq!(refs[1].plugin_id, "other-plugin");
    }

    #[test]
    fn get_returns_not_found_for_unknown_id() {
        let conn = migrated_connection();
        let result = get(&conn, 999);
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }
}
