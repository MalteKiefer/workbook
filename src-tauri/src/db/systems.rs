use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::audit_log;
use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct System {
    pub id: i64,
    pub customer_id: i64,
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
    pub archived_at_utc: Option<String>,
    pub archived_at_tz: Option<String>,
    pub maintenance_interval_days: Option<i64>,
    pub operating_system: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewSystem {
    pub customer_id: i64,
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
    /// `#[serde(default)]` -- every call site that constructs this from
    /// JavaScript before this field existed (all 17 plugin sections'
    /// create-and-link/adopt flows, both Journal pickers) omits the key
    /// entirely rather than sending `null`; without this, serde treats an
    /// absent `Option<T>` key as a hard deserialize error, not `None`.
    #[serde(default)]
    pub maintenance_interval_days: Option<i64>,
    /// `#[serde(default)]` -- same reasoning as `maintenance_interval_days`
    /// above: every call site that constructs this from JavaScript before
    /// this field existed (all 17 plugin sections' create-and-link/adopt
    /// flows, both Journal pickers, CSV import, the network scan panel's
    /// create-system flow) omits the key entirely rather than sending
    /// `null`; without this, serde treats an absent `Option<T>` key as a
    /// hard deserialize error, not `None`.
    #[serde(default)]
    pub operating_system: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateSystem {
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
    /// `#[serde(default)]` -- every call site that constructs this from
    /// JavaScript before this field existed (all 17 plugin sections'
    /// create-and-link/adopt flows, both Journal pickers) omits the key
    /// entirely rather than sending `null`; without this, serde treats an
    /// absent `Option<T>` key as a hard deserialize error, not `None`.
    #[serde(default)]
    pub maintenance_interval_days: Option<i64>,
    /// `#[serde(default)]` -- same reasoning as `maintenance_interval_days`
    /// above: every call site that constructs this from JavaScript before
    /// this field existed (all 17 plugin sections' create-and-link/adopt
    /// flows, both Journal pickers, CSV import, the network scan panel's
    /// create-system flow) omits the key entirely rather than sending
    /// `null`; without this, serde treats an absent `Option<T>` key as a
    /// hard deserialize error, not `None`.
    #[serde(default)]
    pub operating_system: Option<String>,
}

fn row_to_system(row: &Row) -> rusqlite::Result<System> {
    Ok(System {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        name: row.get("name")?,
        system_type: row.get("system_type")?,
        hostname: row.get("hostname")?,
        ip_address: row.get("ip_address")?,
        notes: row.get("notes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
        archived_at_utc: row.get("archived_at_utc")?,
        archived_at_tz: row.get("archived_at_tz")?,
        maintenance_interval_days: row.get("maintenance_interval_days")?,
        operating_system: row.get("operating_system")?,
    })
}

/// A non-positive interval would make `maintenance::is_overdue` treat the
/// system as permanently overdue (any elapsed time satisfies `>= 0` or
/// `>= negative days`), silently, with no error surfaced anywhere -- reject
/// it here instead of leaving that as a confusing dashboard artifact.
fn validate_maintenance_interval(days: Option<i64>) -> Result<(), AppError> {
    match days {
        Some(n) if n <= 0 => Err(AppError::Validation(format!(
            "Wartungsintervall muss positiv sein, nicht {n} Tage."
        ))),
        _ => Ok(()),
    }
}

pub fn create(conn: &Connection, input: NewSystem, tz: &Tz) -> Result<System, AppError> {
    validate_maintenance_interval(input.maintenance_interval_days)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO systems (customer_id, name, system_type, hostname, ip_address, notes, maintenance_interval_days, operating_system, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?9, ?10)",
        params![input.customer_id, input.name, input.system_type, input.hostname, input.ip_address, input.notes, input.maintenance_interval_days, input.operating_system, now_utc, now_tz],
    )?;
    let system = get(conn, conn.last_insert_rowid())?;
    if let Err(e) = audit_log::record(
        conn,
        "system",
        system.id,
        "created",
        &format!("System \"{}\" angelegt", system.name),
        tz,
    ) {
        eprintln!("Audit-Log-Eintrag konnte nicht gespeichert werden: {e}");
    }
    Ok(system)
}

pub fn get(conn: &Connection, id: i64) -> Result<System, AppError> {
    conn.query_row(
        "SELECT * FROM systems WHERE id = ?1",
        params![id],
        row_to_system,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("System {id} nicht gefunden")))
}

pub fn list_by_customer(
    conn: &Connection,
    customer_id: i64,
    include_archived: bool,
) -> Result<Vec<System>, AppError> {
    let sql = if include_archived {
        "SELECT * FROM systems WHERE customer_id = ?1 ORDER BY name COLLATE NOCASE"
    } else {
        "SELECT * FROM systems WHERE customer_id = ?1 AND archived_at_utc IS NULL ORDER BY name COLLATE NOCASE"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![customer_id], row_to_system)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(
    conn: &Connection,
    id: i64,
    input: UpdateSystem,
    tz: &Tz,
) -> Result<System, AppError> {
    validate_maintenance_interval(input.maintenance_interval_days)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE systems SET name = ?1, system_type = ?2, hostname = ?3, ip_address = ?4, notes = ?5, maintenance_interval_days = ?6, operating_system = ?7, updated_at_utc = ?8, updated_at_tz = ?9 WHERE id = ?10",
        params![input.name, input.system_type, input.hostname, input.ip_address, input.notes, input.maintenance_interval_days, input.operating_system, now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("System {id} nicht gefunden")));
    }
    let system = get(conn, id)?;
    if let Err(e) = audit_log::record(
        conn,
        "system",
        system.id,
        "updated",
        &format!("System \"{}\" bearbeitet", system.name),
        tz,
    ) {
        eprintln!("Audit-Log-Eintrag konnte nicht gespeichert werden: {e}");
    }
    Ok(system)
}

/// Sets ONLY `maintenance_interval_days` on one system, leaving every other
/// field untouched -- deliberately narrower than `update()`, which requires
/// the caller to already have and resend every other field. Used by the
/// bulk "Wartungsintervall setzen" action, where the frontend only has each
/// selected system's id, not its full current field set.
pub fn set_maintenance_interval(
    conn: &Connection,
    id: i64,
    maintenance_interval_days: Option<i64>,
    tz: &Tz,
) -> Result<System, AppError> {
    validate_maintenance_interval(maintenance_interval_days)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE systems SET maintenance_interval_days = ?1, updated_at_utc = ?2, updated_at_tz = ?3 WHERE id = ?4",
        params![maintenance_interval_days, now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("System {id} nicht gefunden")));
    }
    let system = get(conn, id)?;
    if let Err(e) = audit_log::record(
        conn,
        "system",
        system.id,
        "updated",
        &format!("Wartungsintervall von \"{}\" geändert", system.name),
        tz,
    ) {
        eprintln!("Audit-Log-Eintrag konnte nicht gespeichert werden: {e}");
    }
    Ok(system)
}

pub fn archive(conn: &Connection, id: i64, tz: &Tz) -> Result<(), AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    // Fetched before archiving purely to include the system's name in the
    // audit-log summary; the archive itself doesn't need this row.
    let name = get(conn, id).ok().map(|s| s.name);
    let changed = conn.execute(
        "UPDATE systems SET archived_at_utc = ?1, archived_at_tz = ?2 WHERE id = ?3",
        params![now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("System {id} nicht gefunden")));
    }
    let summary = match name {
        Some(name) => format!("System \"{name}\" archiviert"),
        None => format!("System #{id} archiviert"),
    };
    if let Err(e) = audit_log::record(conn, "system", id, "archived", &summary, tz) {
        eprintln!("Audit-Log-Eintrag konnte nicht gespeichert werden: {e}");
    }
    Ok(())
}

/// Latest `performed_at_utc` among a system's entries, or `None` if it has
/// none yet. `MAX()` over zero matching rows returns SQL `NULL`, which
/// `rusqlite` maps to `Ok(None)` for an `Option<String>` column -- this
/// does not need an `.optional()` wrapper the way `query_row` + `NOT NULL`
/// lookups elsewhere in this module do.
pub fn latest_performed_at(conn: &Connection, system_id: i64) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT MAX(performed_at_utc) FROM entries WHERE system_id = ?1",
        params![system_id],
        |row| row.get(0),
    )
    .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::customers::{self, NewCustomer};
    use crate::db::test_support::migrated_connection;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    // Regression test: every plugin section's create-and-link/adopt flow
    // (17 plugins) and both Journal pickers build this JSON payload without
    // a `maintenance_interval_days` key at all -- they predate the field
    // and were never updated. Without `#[serde(default)]` on the field,
    // this fails to deserialize with "missing field
    // `maintenance_interval_days`" instead of defaulting to `None`.
    #[test]
    fn new_system_deserializes_without_maintenance_interval_days_key() {
        let json = r#"{
            "customer_id": 1,
            "name": "web-01",
            "system_type": "",
            "hostname": "",
            "ip_address": "",
            "notes": ""
        }"#;
        let parsed: NewSystem = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.maintenance_interval_days, None);
    }

    #[test]
    fn update_system_deserializes_without_maintenance_interval_days_key() {
        let json = r#"{
            "name": "web-01",
            "system_type": "",
            "hostname": "",
            "ip_address": "",
            "notes": ""
        }"#;
        let parsed: UpdateSystem = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.maintenance_interval_days, None);
    }

    fn seed_customer(conn: &Connection) -> i64 {
        customers::create(
            conn,
            NewCustomer {
                name: "ACME".into(),
                short_code: "ACME".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "Server".into(),
                hostname: "fs01.acme.local".into(),
                ip_address: "10.0.0.5".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(created.hostname, "fs01.acme.local");
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn create_rejects_zero_or_negative_maintenance_interval() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        for bad_interval in [0, -1, -30] {
            let result = create(
                &conn,
                NewSystem {
                    customer_id,
                    name: "FS01".into(),
                    system_type: "".into(),
                    hostname: "".into(),
                    ip_address: "".into(),
                    notes: "".into(),
                    maintenance_interval_days: Some(bad_interval),
                    operating_system: None,
                },
                &berlin(),
            );
            assert!(
                result.is_err(),
                "{bad_interval} Tage hätte abgelehnt werden müssen"
            );
        }
    }

    #[test]
    fn update_rejects_zero_or_negative_maintenance_interval() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        let result = update(
            &conn,
            created.id,
            UpdateSystem {
                name: "FS01".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: Some(-5),
                operating_system: None,
            },
            &berlin(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn set_maintenance_interval_changes_only_the_interval() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "Server".into(),
                hostname: "fs01.acme.local".into(),
                ip_address: "10.0.0.5".into(),
                notes: "Ein paar Notizen".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        let updated = set_maintenance_interval(&conn, created.id, Some(30), &berlin()).unwrap();
        assert_eq!(updated.maintenance_interval_days, Some(30));
        assert_eq!(updated.name, created.name);
        assert_eq!(updated.system_type, created.system_type);
        assert_eq!(updated.hostname, created.hostname);
        assert_eq!(updated.ip_address, created.ip_address);
        assert_eq!(updated.notes, created.notes);
        assert_eq!(updated.created_at_utc, created.created_at_utc);
    }

    #[test]
    fn set_maintenance_interval_rejects_zero_or_negative_maintenance_interval() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        for bad_interval in [0, -1, -30] {
            let result = set_maintenance_interval(&conn, created.id, Some(bad_interval), &berlin());
            assert!(
                result.is_err(),
                "{bad_interval} Tage hätte abgelehnt werden müssen"
            );
        }
    }

    #[test]
    fn set_maintenance_interval_returns_not_found_for_nonexistent_id() {
        let conn = migrated_connection();
        let result = set_maintenance_interval(&conn, 999, Some(30), &berlin());
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[test]
    fn create_rejects_unknown_customer() {
        let conn = migrated_connection();
        let result = create(
            &conn,
            NewSystem {
                customer_id: 999,
                name: "Ghost".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        );
        assert!(matches!(result, Err(AppError::Database(_))));
    }

    #[test]
    fn list_by_customer_excludes_archived_by_default() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let a = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Aktiv".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        let b = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Alt".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        archive(&conn, b.id, &berlin()).unwrap();

        let active = list_by_customer(&conn, customer_id, false).unwrap();
        assert_eq!(active.iter().map(|s| s.id).collect::<Vec<_>>(), vec![a.id]);
        assert_eq!(list_by_customer(&conn, customer_id, true).unwrap().len(), 2);
    }

    #[test]
    fn update_changes_fields_and_keeps_created_at() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Alt".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        let updated = update(
            &conn,
            created.id,
            UpdateSystem {
                name: "Neu".into(),
                system_type: "Firewall".into(),
                hostname: "fw.acme.local".into(),
                ip_address: "10.0.0.1".into(),
                notes: "".into(),
                maintenance_interval_days: Some(90),
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(updated.name, "Neu");
        assert_eq!(updated.created_at_utc, created.created_at_utc);
        assert_eq!(updated.maintenance_interval_days, Some(90));
    }

    #[test]
    fn create_persists_maintenance_interval_days_and_defaults_to_none() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let without_interval = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Ohne Intervall".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(without_interval.maintenance_interval_days, None);

        let with_interval = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Mit Intervall".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: Some(90),
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(with_interval.maintenance_interval_days, Some(90));
        assert_eq!(
            get(&conn, with_interval.id)
                .unwrap()
                .maintenance_interval_days,
            Some(90)
        );
    }

    #[test]
    fn create_persists_operating_system_and_defaults_to_none() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let without_os = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Ohne Betriebssystem".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(without_os.operating_system, None);

        let mut input = NewSystem {
            customer_id,
            name: "Mit Betriebssystem".into(),
            system_type: "".into(),
            hostname: "".into(),
            ip_address: "".into(),
            notes: "".into(),
            maintenance_interval_days: None,
            operating_system: None,
        };
        input.operating_system = Some("Windows 11 Pro".to_string());
        let created = create(&conn, input, &berlin()).unwrap();
        assert_eq!(created.operating_system, Some("Windows 11 Pro".to_string()));
        assert_eq!(
            get(&conn, created.id).unwrap().operating_system,
            Some("Windows 11 Pro".to_string())
        );
    }

    #[test]
    fn update_can_set_and_clear_operating_system() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(created.operating_system, None);

        let mut update_input = UpdateSystem {
            name: created.name.clone(),
            system_type: created.system_type.clone(),
            hostname: created.hostname.clone(),
            ip_address: created.ip_address.clone(),
            notes: created.notes.clone(),
            maintenance_interval_days: created.maintenance_interval_days,
            operating_system: Some("Ubuntu 24.04".to_string()),
        };
        let updated = update(&conn, created.id, update_input.clone(), &berlin()).unwrap();
        assert_eq!(updated.operating_system, Some("Ubuntu 24.04".to_string()));

        update_input.operating_system = None;
        let cleared = update(&conn, created.id, update_input, &berlin()).unwrap();
        assert_eq!(cleared.operating_system, None);
    }

    #[test]
    fn latest_performed_at_is_none_without_entries() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let system = create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();

        assert_eq!(latest_performed_at(&conn, system.id).unwrap(), None);
    }

    #[test]
    fn latest_performed_at_returns_the_most_recent_entry() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let system = create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();

        for performed_at in [
            "2026-01-01T12:00:00.000Z",
            "2026-06-01T12:00:00.000Z",
            "2026-03-01T12:00:00.000Z",
        ] {
            conn.execute(
                "INSERT INTO entries (customer_id, system_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
                 VALUES (?1, ?2, 'Wartung', '', 'wartung', ?3, 'Europe/Berlin', ?3, 'Europe/Berlin', ?3, 'Europe/Berlin')",
                params![customer_id, system.id, performed_at],
            )
            .unwrap();
        }

        assert_eq!(
            latest_performed_at(&conn, system.id).unwrap(),
            Some("2026-06-01T12:00:00.000Z".to_string())
        );
    }

    fn audit_log_rows(conn: &Connection, entity_type: &str, action: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE entity_type = ?1 AND action = ?2",
            params![entity_type, action],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn create_records_audit_log_entry() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(audit_log_rows(&conn, "system", "created"), 1);
    }

    #[test]
    fn update_records_audit_log_entry() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Alt".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        update(
            &conn,
            created.id,
            UpdateSystem {
                name: "Neu".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(audit_log_rows(&conn, "system", "updated"), 1);
    }

    #[test]
    fn archive_records_audit_log_entry() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Alt".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
                operating_system: None,
            },
            &berlin(),
        )
        .unwrap();
        archive(&conn, created.id, &berlin()).unwrap();
        assert_eq!(audit_log_rows(&conn, "system", "archived"), 1);
    }
}
