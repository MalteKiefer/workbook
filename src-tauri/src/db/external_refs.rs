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

/// Inserts or updates a link between a local system and an external plugin
/// system -- a system has at most one row per plugin (`system_id`,
/// `plugin_id`). Re-syncing updates `payload_json`/`synced_at_*` in the same
/// row instead of accumulating duplicates.
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
            // Refuse to link this external device to a second, different
            // local system -- this is the exact bug class behind a real
            // incident (a stale frontend cache let a bulk "create and
            // link" run twice, producing duplicate systems all pointing
            // at the same external device). `system_id` here is
            // genuinely new for this `plugin_id` (checked above), so any
            // existing row for this `(plugin_id, external_id)` pair
            // belongs to a different system -- a legitimate re-link to a
            // DIFFERENT system must go through an explicit unlink first
            // (which deletes the old row via `delete` below), never
            // silently through here.
            let conflicting_system_id: Option<i64> = conn
                .query_row(
                    "SELECT system_id FROM external_refs WHERE plugin_id = ?1 AND external_id = ?2",
                    params![plugin_id, external_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(other_system_id) = conflicting_system_id {
                return Err(AppError::Plugin(format!(
                    "Dieses externe Gerät ist bereits mit System #{other_system_id} verknüpft."
                )));
            }
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
    conn.query_row(
        "SELECT * FROM external_refs WHERE id = ?1",
        params![id],
        row_to_external_ref,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("external_ref {id} nicht gefunden")))
}

pub fn list_for_system(conn: &Connection, system_id: i64) -> Result<Vec<ExternalRef>, AppError> {
    let mut stmt =
        conn.prepare("SELECT * FROM external_refs WHERE system_id = ?1 ORDER BY plugin_id")?;
    let rows = stmt.query_map(params![system_id], row_to_external_ref)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// All links for a plugin (e.g. `"ninja:<connection_id>"`), REGARDLESS of
/// which customer the respective linked local system is currently assigned
/// to. For `commands::plugins::sync_ninja_connection`, which must report the
/// "already linked" status of every external device -- correctly even when
/// the Ninja organization was mapped to a different local customer AFTER
/// linking than the one the linked system actually lives under
/// (organization mappings can be changed/corrected at any time, see
/// `NinjaOrgMapping`; per `unmap_ninja_organization`'s own documentation,
/// unmapping an organization is deliberately NOT an automatic unlinking of
/// its already-linked devices -- links persist across mapping changes, so
/// they must be findable independent of the current `customer_id` rather
/// than only being searched for within the systems of ONE customer).
pub fn list_for_plugin(conn: &Connection, plugin_id: &str) -> Result<Vec<ExternalRef>, AppError> {
    let mut stmt =
        conn.prepare("SELECT * FROM external_refs WHERE plugin_id = ?1 ORDER BY external_id")?;
    let rows = stmt.query_map(params![plugin_id], row_to_external_ref)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Deletes the link between a local system and a plugin (e.g. when removing
/// a Ninja link). Not an error if no such row exists -- the result (no link
/// left) is the same.
pub fn delete(conn: &Connection, system_id: i64, plugin_id: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM external_refs WHERE system_id = ?1 AND plugin_id = ?2",
        params![system_id, plugin_id],
    )?;
    Ok(())
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
        seed_system_under_new_customer(conn, "ACME")
    }

    // Like `seed_system`, but under a freshly created customer with the
    // given `short_code` -- for tests that need TWO distinct local systems
    // in the same database (`seed_system` alone can only be called once per
    // test, since `customers.short_code` is UNIQUE).
    fn seed_system_under_new_customer(conn: &Connection, short_code: &str) -> i64 {
        let customer_id = customers::create(
            conn,
            NewCustomer {
                name: short_code.into(),
                short_code: short_code.into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap()
        .id;
        systems::create(
            conn,
            NewSystem {
                customer_id,
                name: format!("{short_code}-FS01"),
                system_type: "Server".into(),
                hostname: format!("fs01.{}.local", short_code.to_lowercase()),
                ip_address: "10.0.0.5".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    #[test]
    fn upsert_twice_for_same_plugin_updates_single_row() {
        let conn = migrated_connection();
        let system_id = seed_system(&conn);

        let first = upsert(
            &conn,
            system_id,
            "dummy",
            "dummy-1",
            r#"{"status":"ok"}"#,
            &berlin(),
        )
        .unwrap();
        let second = upsert(
            &conn,
            system_id,
            "dummy",
            "dummy-1",
            r#"{"status":"changed"}"#,
            &berlin(),
        )
        .unwrap();

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

    // Regression coverage for the exact production incident this file's
    // `upsert` now guards against: a stale frontend cache let a bulk
    // "create and link" run twice, producing a second, different local
    // System that tried to claim an external device ID already linked to
    // the first one. `upsert` must refuse the second link instead of
    // silently inserting a duplicate row.
    #[test]
    fn upsert_refuses_to_link_second_system_to_same_external_id() {
        let conn = migrated_connection();
        let system_a = seed_system(&conn);
        let system_b = seed_system_under_new_customer(&conn, "OTHER");

        upsert(&conn, system_a, "ninja:conn-1", "42", "{}", &berlin()).unwrap();

        let result = upsert(&conn, system_b, "ninja:conn-1", "42", "{}", &berlin());

        assert!(matches!(result, Err(AppError::Plugin(_))));
        let refs = list_for_plugin(&conn, "ninja:conn-1").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].system_id, system_a);
    }

    // The refusal above must not block the LEGITIMATE flow: unlink the
    // external device from system A first (an explicit `delete`), then
    // system B may claim it -- there is no longer any conflicting row.
    #[test]
    fn upsert_allows_relink_to_different_system_after_delete() {
        let conn = migrated_connection();
        let system_a = seed_system(&conn);
        let system_b = seed_system_under_new_customer(&conn, "OTHER");

        upsert(&conn, system_a, "ninja:conn-1", "42", "{}", &berlin()).unwrap();
        delete(&conn, system_a, "ninja:conn-1").unwrap();

        let result = upsert(&conn, system_b, "ninja:conn-1", "42", "{}", &berlin());

        assert!(result.is_ok());
        let refs = list_for_plugin(&conn, "ninja:conn-1").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].system_id, system_b);
    }

    #[test]
    fn get_returns_not_found_for_unknown_id() {
        let conn = migrated_connection();
        let result = get(&conn, 999);
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[test]
    fn delete_removes_the_row_for_that_plugin_only() {
        let conn = migrated_connection();
        let system_id = seed_system(&conn);
        upsert(&conn, system_id, "dummy", "dummy-1", "{}", &berlin()).unwrap();
        upsert(&conn, system_id, "other-plugin", "ext-9", "{}", &berlin()).unwrap();

        delete(&conn, system_id, "dummy").unwrap();

        let refs = list_for_system(&conn, system_id).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].plugin_id, "other-plugin");
    }

    #[test]
    fn delete_is_a_no_op_when_no_matching_row_exists() {
        let conn = migrated_connection();
        let system_id = seed_system(&conn);

        let result = delete(&conn, system_id, "nonexistent-plugin");

        assert!(result.is_ok());
        assert!(list_for_system(&conn, system_id).unwrap().is_empty());
    }

    #[test]
    fn list_for_plugin_only_returns_rows_for_that_plugin() {
        let conn = migrated_connection();
        let system_id = seed_system(&conn);
        upsert(&conn, system_id, "ninja:conn-1", "10", "{}", &berlin()).unwrap();
        upsert(&conn, system_id, "other-plugin", "ext-9", "{}", &berlin()).unwrap();

        let refs = list_for_plugin(&conn, "ninja:conn-1").unwrap();

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].external_id, "10");
        assert_eq!(refs[0].system_id, system_id);
    }

    // Regression coverage for the exact bug this function fixes: a Ninja
    // organization gets mapped to customer A, a device gets linked to a
    // system created under customer A, and the organization is LATER
    // remapped to a different customer B (correcting an earlier mistake).
    // The link itself is untouched by remapping (see this function's own
    // doc comment) -- `list_for_plugin` must still find it by plugin_id
    // alone, without being scoped to either customer's systems, since the
    // caller (`sync_ninja_connection`) no longer knows in advance which
    // customer the linked system lives under.
    #[test]
    fn list_for_plugin_finds_link_regardless_of_the_linked_systems_current_customer() {
        let conn = migrated_connection();
        let customer_a = customers::create(
            &conn,
            NewCustomer {
                name: "Falscher Kunde".into(),
                short_code: "FALSCH".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap()
        .id;
        let customer_b = customers::create(
            &conn,
            NewCustomer {
                name: "Richtiger Kunde".into(),
                short_code: "RICHTIG".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap()
        .id;
        let system_under_a = systems::create(
            &conn,
            NewSystem {
                customer_id: customer_a,
                name: "SRV-01".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap()
        .id;
        upsert(&conn, system_under_a, "ninja:conn-1", "10", "{}", &berlin()).unwrap();

        // Nothing was ever created under customer_b -- the point is that the
        // link is still found even though it doesn't belong to customer_b at
        // all (customer_b stands in for the org's freshly-corrected mapping).
        let _ = customer_b;

        let refs = list_for_plugin(&conn, "ninja:conn-1").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].system_id, system_under_a);
    }
}
